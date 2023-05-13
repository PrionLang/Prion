use crate::ast::{StructDef, StructField, TyParam, TypeApplication, TypePath, TypePathSeg, While};
use crate::{
    ast::{
        expr_as_block, AssignmentOperator, AssignmentTarget, Atom, BinaryOp, Delimiter, Expr,
        FieldVisibility, FuncParam, ItemVisibility, MemberVisibility, Node, Pattern, TypeExpr,
        UnaryOp,
    },
    error::Error,
    span::{Provider, Span, Spanned},
    token::{ChumskyTokenStreamer, StringLiteralFlags, TokenInfo, TokenReader},
};
use chumsky::{
    combinator::{DelimitedBy, IgnoreThen, Repeated, ThenIgnore},
    error::Error as _,
    prelude::{
        choice, end, filter_map, just, recursive, select, Parser as ChumskyParser, Recursive,
    },
    primitive::Just,
    stream::Stream,
};
use std::result::Result as StdResult;

pub type Result<T> = StdResult<T, Error>;
pub type RecursiveParser<'a, T> = Recursive<'a, TokenInfo, T, Error>;
type RecursiveDef<'a, T> = Recursive<'a, TokenInfo, T, Error>;

type JustToken = Just<TokenInfo, TokenInfo, Error>;
type RepeatedToken = Repeated<JustToken>;

type ThenWsTy<T, O> = ThenIgnore<T, RepeatedToken, O, Vec<TokenInfo>>;
type WsThenTy<T, O> = IgnoreThen<RepeatedToken, T, Vec<TokenInfo>, O>;
type PadWsTy<T, O> = ThenWsTy<WsThenTy<T, O>, O>;
type DelimitedTy<A> = DelimitedBy<
    A,
    ThenWsTy<JustToken, TokenInfo>,
    WsThenTy<JustToken, TokenInfo>,
    TokenInfo,
    TokenInfo,
>;

pub trait TokenParser<O> = ChumskyParser<TokenInfo, O, Error = Error>;

trait WsPadExt<T, O> {
    fn then_ws(self) -> ThenWsTy<T, O>;
    fn ws_then(self) -> WsThenTy<T, O>;
    fn pad_ws(self) -> PadWsTy<T, O>;
    fn delimited(self, delimiter: Delimiter) -> DelimitedTy<Self>
    where
        Self: Sized;
}

impl<O, T: ChumskyParser<TokenInfo, O, Error = Error>> WsPadExt<T, O> for T {
    #[inline]
    fn then_ws(self) -> ThenWsTy<T, O> {
        self.then_ignore(just(TokenInfo::Whitespace).repeated())
    }

    #[inline]
    fn ws_then(self) -> WsThenTy<T, O> {
        just(TokenInfo::Whitespace).repeated().ignore_then(self)
    }

    #[inline] // Maybe inline is a bad idea
    fn pad_ws(self) -> PadWsTy<T, O> {
        self.padded_by(just(TokenInfo::Whitespace).repeated())
    }

    #[inline]
    fn delimited(self, delimiter: Delimiter) -> DelimitedTy<Self>
    where
        Self: Sized,
    {
        self.delimited_by(
            just(delimiter.open_token()).then_ws(),
            just(delimiter.close_token()).ws_then(),
        )
    }
}

macro_rules! kw {
    ($($kw:literal)|+) => {{
        just([$(TokenInfo::Ident($kw.to_string(), false)),+]).map_with_span(Spanned)
    }};
    ($kw:ident) => {{
        kw!(stringify!($kw))
    }};
    (@pad $kw:tt) => {{
        kw!($kw).pad_ws()
    }};
}

/// Resolves escape sequences in a string.
fn resolve_string(content: String, flags: StringLiteralFlags, span: Span) -> Result<String> {
    if flags.is_raw() {
        return Ok(content);
    }

    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars();
    let mut pos = span.start;

    while let Some(mut c) = chars.next() {
        if c == '\\' {
            pos += 1;

            macro_rules! hex_sequence {
                ($length:literal) => {{
                    let sequence = chars.by_ref().take($length).collect::<String>();
                    let value = u32::from_str_radix(&sequence, 16).map_err(|_| {
                        Error::invalid_hex_escape_sequence(
                            sequence.clone(),
                            span.get_span(pos - 1, pos + 1 + $length),
                        )
                    })?;

                    pos += $length;
                    char::from_u32(value).ok_or(Error::invalid_hex_escape_sequence(
                        sequence,
                        span.get_span(pos - 1, pos + 1 + $length),
                    ))?
                }};
            }

            c = match chars.next().ok_or_else(|| {
                Error::unexpected_eof(
                    Span::single(span.src, pos),
                    Some(('n', "insert an escape sequence")),
                )
            })? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\x08',
                'f' => '\x0c',
                '0' => '\0',
                '\'' => '\'',
                '"' => '"',
                '\\' => '\\',
                'x' => hex_sequence!(2),
                'u' => hex_sequence!(4),
                'U' => hex_sequence!(8),
                c => {
                    return Err(Error::unknown_escape_sequence(
                        c,
                        span.get_span(pos - 1, pos + 1),
                    ))
                }
            };
        }

        result.push(c);
        pos += 1;
    }

    Ok(result)
}

pub fn item_vis_parser<'a>() -> impl TokenParser<ItemVisibility> + Clone + 'a {
    kw!("private")
        .to(ItemVisibility::Mod)
        .or(kw!("public").ignore_then(
            choice((
                kw!("mod").to(ItemVisibility::Mod),
                kw!("super").to(ItemVisibility::Super),
                kw!("lib").to(ItemVisibility::Lib),
            ))
            .delimited(Delimiter::Paren)
            .ws_then()
            .or_not()
            .map(|v| v.unwrap_or(ItemVisibility::Public)),
        ))
        .or_not()
        .map(|v| v.unwrap_or(ItemVisibility::Mod))
        .then_ws()
}

pub fn member_vis_parser<'a>() -> impl TokenParser<Spanned<MemberVisibility>> + Clone + 'a {
    kw!("private")
        .to(MemberVisibility::Private)
        .or(kw!("public").ignore_then(
            choice((
                kw!("sub").to(MemberVisibility::Sub),
                kw!("mod").to(MemberVisibility::Mod),
                kw!("super").to(MemberVisibility::Super),
                kw!("lib").to(MemberVisibility::Lib),
            ))
            .delimited(Delimiter::Paren)
            .ws_then()
            .or_not()
            .map(|v| v.unwrap_or(MemberVisibility::Public)),
        ))
        .or_not()
        .map(|v| v.unwrap_or(MemberVisibility::Mod))
        .map_with_span(Spanned)
        .then_ws()
}

pub fn field_vis_parser<'a>() -> impl TokenParser<Spanned<FieldVisibility>> + Clone + 'a {
    const FALLBACK: (MemberVisibility, Option<Span>) = (MemberVisibility::Private, None);

    #[inline]
    fn vis_map(
        Spanned(v, span): Spanned<Option<MemberVisibility>>,
    ) -> (MemberVisibility, Option<Span>) {
        (v.unwrap_or(MemberVisibility::Public), Some(span))
    }

    let vis = choice((
        kw!("private").to(MemberVisibility::Private),
        kw!("sub").to(MemberVisibility::Sub),
        kw!("mod").to(MemberVisibility::Mod),
        kw!("super").to(MemberVisibility::Super),
        kw!("lib").to(MemberVisibility::Lib),
    ))
    .or_not()
    .then_ws();

    let get_vis = vis
        .clone()
        .then_ignore(kw!("get"))
        .map_with_span(Spanned)
        .map(vis_map);

    let set_vis = vis
        .clone()
        .then_ignore(kw!("set"))
        .map_with_span(Spanned)
        .map(vis_map);

    let one_of = get_vis
        .clone()
        .map(|get| (get, FALLBACK))
        .or(set_vis.clone().map(|set| (FALLBACK, set)));
    let combo = get_vis
        .then_ignore(just(TokenInfo::Comma).pad_ws())
        .then(set_vis);

    let unexpanded = member_vis_parser().map(|vis| {
        Spanned(
            FieldVisibility {
                get: (vis.0, None),
                set: (vis.0, None),
            },
            vis.1,
        )
    });
    let expanded = kw!("public")
        .ignore_then(combo.or(one_of).delimited(Delimiter::Paren))
        .map_with_span(|(get, set), span| Spanned(FieldVisibility { get, set }, span));

    expanded.or(unexpanded).then_ws()
}

pub fn type_expr_parser<'a>() -> RecursiveParser<'a, Spanned<TypeExpr>> {
    recursive::<_, Spanned<_>, _, _, _>(|ty| {
        let infer_ty = select!(TokenInfo::Ident(i, false) if i == "_" => TypeExpr::Infer)
            .map_with_span(Spanned)
            .pad_ws();

        let ty_application = select!(TokenInfo::Ident(name, _) => name)
            .map_with_span(Spanned)
            .then_ignore(just(TokenInfo::Equals).pad_ws())
            .or_not()
            .then(ty.clone())
            .separated_by(just(TokenInfo::Comma).pad_ws())
            .at_least(1)
            .allow_trailing()
            .delimited(Delimiter::Angle)
            .try_map(|mut args, _span| {
                let partition = args
                    .iter()
                    .position(|(name, _)| name.is_some())
                    .unwrap_or(args.len());
                let kwargs = args.split_off(partition);

                Ok(TypeApplication {
                    args: args.into_iter().map(|(_, ty)| ty).collect(),
                    kwargs: kwargs
                        .into_iter()
                        .map(|(name, ty)| {
                            Ok((
                                name.ok_or_else(|| {
                                    Error::unexpected_positional_argument(ty.span())
                                })?,
                                ty,
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?,
                })
            })
            .map_with_span(Spanned)
            .or_not();

        let path = select! { |span|
            TokenInfo::Ident(name, _) => Spanned(name, span),
        }
        .then(ty_application)
        .map(|(ident, app)| TypePathSeg(ident, app))
        .map_with_span(Spanned)
        .separated_by(just(TokenInfo::Dot).pad_ws())
        .at_least(1)
        .map(TypePath)
        .map_with_span(Spanned)
        .map(TypeExpr::Path)
        .map_with_span(Spanned)
        .pad_ws();

        let tuple = ty
            .clone()
            .separated_by(just(TokenInfo::Comma).pad_ws())
            .allow_trailing()
            .delimited(Delimiter::Paren)
            .map(TypeExpr::Tuple)
            .map_with_span(Spanned)
            .pad_ws()
            .boxed();

        let array = ty
            .clone()
            .map(Box::new)
            .then_ignore(just(TokenInfo::Semicolon).pad_ws())
            .then(
                select! {
                    TokenInfo::IntLiteral(value, info) => Atom::Int(value, info),
                    TokenInfo::Ident(name, _) => Atom::Ident(name),
                }
                .map_with_span(Spanned)
                .or_not(),
            )
            .delimited(Delimiter::Bracket)
            .map(|(ty, size)| TypeExpr::Array(ty, size))
            .map_with_span(Spanned)
            .pad_ws()
            .boxed();

        let ty_atom = choice((
            ty.clone().delimited(Delimiter::Paren),
            infer_ty,
            path,
            tuple,
            array,
        ));

        ty_atom
    })
}

/// A block label, e.g. `:a`
pub fn block_label<'a>() -> impl TokenParser<Spanned<String>> + Clone + 'a {
    just(TokenInfo::Colon)
        .ignore_then(select!(TokenInfo::Ident(name, _) => name))
        .map_with_span(Spanned)
        .labelled("block label")
}

/// A pattern to match against in a declaration.
pub fn pat_parser<'a>() -> RecursiveParser<'a, Spanned<Pattern>> {
    recursive(|pat| {
        let ident = kw!("mut")
            .then_ws()
            .or_not()
            .then(select!(TokenInfo::Ident(name, _) => name).map_with_span(Spanned))
            .map_with_span(|(mut_kw, ident), span| {
                Spanned(
                    Pattern::Ident {
                        ident,
                        mut_kw: mut_kw.map(|kw| kw.span()),
                    },
                    span,
                )
            })
            .labelled("identifier")
            .boxed();

        ident
    })
}

type ParamList = Vec<Spanned<FuncParam>>;

#[derive(Clone)]
struct FuncHeader {
    name: Spanned<String>,
    params: ParamList,
    kw_params: ParamList,
    ret: Option<Spanned<TypeExpr>>,
}

#[derive(Clone)]
enum FuncParamOrSep {
    FuncParam(Spanned<FuncParam>),
    Sep(Span),
}

fn split_func_params(
    hspan: Span,
    mut params: Vec<FuncParamOrSep>,
) -> Result<(ParamList, ParamList)> {
    let partition = params
        .iter()
        .position(|p| matches!(p, FuncParamOrSep::Sep(_)))
        .unwrap_or(params.len());
    let kw_params = params.split_off(partition);

    Ok((
        params
            .into_iter()
            .map(|p| {
                if let FuncParamOrSep::FuncParam(p) = p {
                    p
                } else {
                    // SAFETY: `partition` is the index of the first separator, so
                    // there will be no separators before `partition`.
                    unsafe { std::hint::unreachable_unchecked() }
                }
            })
            .collect(),
        kw_params
            .into_iter()
            .skip(1)
            .map(|p| match p {
                FuncParamOrSep::FuncParam(p) => match p.value().pat.value() {
                    Pattern::Ident { .. } => Ok(p),
                    _ => Err(Error::keyword_parameter_not_ident(hspan, p.span())),
                },
                FuncParamOrSep::Sep(span) => {
                    Err(Error::multiple_keyword_parameter_separators(hspan, span))
                }
            })
            .collect::<Result<Vec<_>>>()?,
    ))
}

/// A body parser, i.e. a list of statements.
#[allow(clippy::too_many_lines)]
pub fn body_parser<'a>() -> RecursiveParser<'a, Vec<Spanned<Node>>> {
    let ty = type_expr_parser();
    let pat = pat_parser();
    let block_label = block_label().pad_ws().or_not();
    let item_vis = item_vis_parser();
    let field_vis = field_vis_parser();

    recursive(move |body: RecursiveDef<Vec<Spanned<Node>>>| {
        let expr = expr_parser(body.clone());

        let ident = select! {
            TokenInfo::Ident(name, _) => name,
        }
        .map_with_span(Spanned)
        .pad_ws();

        // Expression as a statement, i.e. `f();`;
        let expression = expr
            .clone()
            .then_ignore(just(TokenInfo::Semicolon).pad_ws())
            .or(brace_ending_expr(expr.clone(), body.clone()).then_ignore(
                // FIXME: is this the best way to disambiguate?
                //  there may be a better way with lazy parsing
                just(TokenInfo::RightBrace)
                    .pad_ws()
                    .ignored()
                    .or(end())
                    .not()
                    .rewind(),
            ))
            .map(Node::Expr)
            .map_with_span(Spanned)
            .labelled("expression");

        // `let` declaration, for example `let x: int32 = 0;`
        let let_decl = kw!("let")
            .map_with_span(|_, span| span)
            .then_ws()
            .then(pat.clone())
            .then(
                just(TokenInfo::Colon)
                    .pad_ws()
                    .ignore_then(ty.clone())
                    .or_not(),
            )
            .then(
                just(TokenInfo::Equals)
                    .pad_ws()
                    .ignore_then(expr.clone())
                    .or_not(),
            )
            .then_ignore(just(TokenInfo::Semicolon))
            .map_with_span(|(((kw_span, pat), ty), value), span| {
                Spanned(
                    Node::Let {
                        kw: kw_span,
                        pat,
                        ty,
                        value,
                    },
                    span,
                )
            })
            .labelled("let declaration")
            .boxed();

        // `const` declaration, for example `const ZERO: int32 = 0;`
        let const_decl = item_vis
            .clone()
            .then(kw!("const").map_with_span(|_, span| span))
            .then(ident.clone())
            .then(
                just(TokenInfo::Colon)
                    .pad_ws()
                    .ignore_then(ty.clone())
                    .or_not(),
            )
            .then(just(TokenInfo::Equals).pad_ws().ignore_then(expr.clone()))
            .then_ignore(just(TokenInfo::Semicolon))
            .map_with_span(|((((vis, kw_span), name), ty), value), span| {
                Spanned(
                    Node::Const {
                        vis,
                        kw: kw_span,
                        name,
                        ty,
                        value,
                    },
                    span,
                )
            })
            .labelled("const declaration")
            .boxed();

        // Single function parameter
        let func_param = pat
            .then_ignore(just(TokenInfo::Colon).then_ws())
            .then(ty.clone())
            .then(
                just(TokenInfo::Equals)
                    .pad_ws()
                    .ignore_then(expr.clone())
                    .or_not(),
            )
            .map_with_span(|((pat, ty), default), span| {
                Spanned(FuncParam { pat, ty, default }, span)
            });

        // Function header starting from "func", i.e. `func f()`
        let func_header = kw!("func")
            .ignore_then(ident.clone())
            .then(
                just(TokenInfo::Asterisk)
                    .map_with_span(|_, span| FuncParamOrSep::Sep(span))
                    .or(func_param.map(FuncParamOrSep::FuncParam))
                    .separated_by(just(TokenInfo::Comma).pad_ws())
                    .allow_trailing()
                    .delimited(Delimiter::Paren),
            )
            .then(
                just([TokenInfo::Minus, TokenInfo::Gt])
                    .pad_ws()
                    .ignore_then(ty.clone())
                    .or_not(),
            )
            .try_map(|((name, params), ty), span| {
                let (params, kw_params) = split_func_params(span, params)?;
                Ok(FuncHeader {
                    name,
                    params,
                    kw_params,
                    ret: ty,
                })
            });

        // Function, i.e. `func f() {}` or `func f() = expr;`
        let func = item_vis
            .clone()
            .then(func_header)
            .then(
                body.delimited(Delimiter::Brace)
                    .map_with_span(Spanned)
                    .pad_ws()
                    .then_ignore(just(TokenInfo::Semicolon).or_not())
                    .or(just(TokenInfo::Equals)
                        .pad_ws()
                        .ignore_then(expr.clone())
                        .then_ignore(just(TokenInfo::Semicolon))
                        .map(expr_as_block)
                        .pad_ws()),
            )
            .try_map(
                |(
                    (
                        vis,
                        FuncHeader {
                            name,
                            params,
                            kw_params,
                            ret,
                        },
                    ),
                    body,
                ),
                 span| {
                    Ok(Spanned(
                        Node::Func {
                            vis,
                            name,
                            params,
                            kw_params,
                            body,
                            ret,
                        },
                        span,
                    ))
                },
            )
            .pad_ws()
            .labelled("function")
            .boxed();

        let control_flow_if_stmt = kw!("if").pad_ws().ignore_then(expr.clone()).or_not();

        // Break statement, i.e. `break;`, `break 5;`, or `break :a;`
        let break_stmt = kw!("break")
            .then(block_label.clone())
            .then(expr.clone().or_not())
            .then(control_flow_if_stmt.clone())
            .then_ignore(just(TokenInfo::Semicolon))
            .map_with_span(|(((kw, label), value), cond), span| {
                Spanned(
                    Node::Break {
                        kw: kw.span(),
                        label,
                        value,
                        cond,
                    },
                    span,
                )
            })
            .pad_ws()
            .labelled("break")
            .boxed();

        // Continue statement, i.e. `continue;` or `continue :a;`
        let continue_stmt = kw!("continue")
            .then(block_label.clone())
            .then(control_flow_if_stmt.clone())
            .then_ignore(just(TokenInfo::Semicolon))
            .map_with_span(|((kw, label), cond), span| {
                Spanned(
                    Node::Continue {
                        kw: kw.span(),
                        label,
                        cond,
                    },
                    span,
                )
            })
            .pad_ws()
            .labelled("continue")
            .boxed();

        // Return statement, i.e. `return;`
        let return_stmt = kw!("return")
            .then_ws()
            .then(expr.clone().or_not())
            .then(control_flow_if_stmt.clone())
            .then_ignore(just(TokenInfo::Semicolon))
            .map_with_span(|((kw, value), cond), span| {
                Spanned(
                    Node::Return {
                        kw: kw.span(),
                        value,
                        cond,
                    },
                    span,
                )
            })
            .pad_ws()
            .labelled("return")
            .boxed();

        let type_params = ident
            .clone()
            .then(
                just(TokenInfo::Colon)
                    .pad_ws()
                    .ignore_then(ty.clone())
                    .or_not(),
            )
            .map(|(name, bound)| TyParam { name, bound })
            .separated_by(just(TokenInfo::Comma).pad_ws())
            .allow_trailing()
            .delimited(Delimiter::Angle)
            .pad_ws()
            .or_not();

        let struct_field = field_vis
            .then(ident.clone())
            .then_ignore(just(TokenInfo::Colon).then_ws())
            .then(ty.clone())
            .then(
                just(TokenInfo::Equals)
                    .pad_ws()
                    .ignore_then(expr.clone())
                    .or_not(),
            )
            .map_with_span(|(((vis, name), ty), default), span| {
                Spanned(
                    StructField {
                        vis,
                        name,
                        ty,
                        default,
                    },
                    span,
                )
            });

        // Struct declaration, i.e. `struct Foo { ... }`
        let struct_decl = item_vis
            .then(kw!("struct").then_ws().ignore_then(ident.clone()))
            .then(type_params)
            .then(
                just(TokenInfo::Colon)
                    .pad_ws()
                    .ignore_then(ty.clone())
                    .or_not(),
            )
            .then(
                struct_field
                    .separated_by(just(TokenInfo::Comma).pad_ws())
                    .allow_trailing()
                    .delimited(Delimiter::Brace),
            )
            .then_ignore(just(TokenInfo::Semicolon).or_not())
            .map_with_span(|((((vis, name), ty_params), extends), fields), span| {
                Spanned(
                    Node::Struct(StructDef {
                        vis,
                        name,
                        ty_params: ty_params.unwrap_or_else(Vec::new),
                        fields,
                        extends,
                    }),
                    span,
                )
            })
            .pad_ws()
            .labelled("struct declaration")
            .boxed();

        choice((
            let_decl,
            const_decl,
            func,
            struct_decl,
            break_stmt,
            continue_stmt,
            return_stmt,
            expression,
        ))
        .pad_ws()
        .repeated()
        .then(
            expr.map(|value| {
                let span = value.span();
                Spanned(Node::ImplicitReturn(value), span)
            })
            .or_not(),
        )
        .map(|(mut nodes, implicit_return)| {
            nodes.extend(implicit_return);
            nodes
        })
    })
}

/// Parses expressions that end with a brace, i.e. `if cond { ... }`
/// These expressions are special because they can be written without a semicolon at the end.
pub fn brace_ending_expr<'a>(
    expr: RecursiveDef<'a, Spanned<Expr>>,
    body: RecursiveDef<'a, Vec<Spanned<Node>>>,
) -> impl ChumskyParser<TokenInfo, Spanned<Expr>, Error = Error> + Clone + 'a {
    type BlockFn = fn(Option<Spanned<String>>, Spanned<Vec<Spanned<Node>>>) -> Expr;

    let braced_body = body
        .delimited(Delimiter::Brace)
        .map_with_span(Spanned)
        .pad_ws();

    let block_label = block_label()
        .then_ignore(just(TokenInfo::Whitespace).repeated())
        .or_not();

    // Braced if-statement, i.e. if cond { ... }
    let braced_if = block_label
        .clone()
        .then_ignore(kw!("if"))
        .then(expr.clone())
        .then(braced_body.clone())
        // else if cond { ... }
        .then(
            kw!("else")
                .ignore_then(kw!("if").pad_ws())
                .ignore_then(expr.clone())
                .then(braced_body.clone())
                .repeated(),
        )
        // else { ... } ...note that this is parsed separately from the else-if
        // since that is repeated and we dont want else {} else {} to be valid.
        .then(kw!("else").ignore_then(braced_body.clone()).or_not())
        .map_with_span(|((((label, cond), body), elif), else_body), span| {
            Spanned(
                Expr::If {
                    label,
                    cond: Box::new(cond),
                    body,
                    else_if_bodies: elif,
                    else_body,
                    ternary: false,
                },
                span,
            )
        })
        .pad_ws()
        .labelled("if-statement")
        .boxed();

    // While-loop, i.e. while cond { ... }
    let while_loop = block_label
        .clone()
        .then_ignore(kw!("while"))
        .then(expr)
        .then(braced_body.clone())
        .then(kw!("else").ignore_then(braced_body.clone()).or_not())
        .map_with_span(|(((label, cond), body), else_body), span| {
            Spanned(
                Expr::While(While {
                    label,
                    cond: Box::new(cond),
                    body,
                    else_body,
                }),
                span,
            )
        })
        .pad_ws()
        .labelled("while-loop")
        .boxed();

    let simple_block = |f: BlockFn| move |(label, body), span| Spanned(f(label, body), span);

    // Loop-expression, i.e. loop { ... }
    let loop_expr = block_label
        .clone()
        .then_ignore(kw!("loop"))
        .then(braced_body.clone())
        .map_with_span(simple_block(|label, body| Expr::Loop { label, body }))
        .pad_ws()
        .labelled("loop-expression")
        .boxed();

    // Standard block, i.e. { ... }
    let block_expr = block_label
        .then(braced_body)
        .map_with_span(simple_block(|label, body| Expr::Block { label, body }))
        .pad_ws()
        .labelled("block-expression")
        .boxed();

    choice((braced_if, while_loop, loop_expr, block_expr))
}

/// Parser to parse an expression.
#[allow(clippy::too_many_lines)]
pub fn expr_parser(body: RecursiveDef<Vec<Spanned<Node>>>) -> RecursiveParser<Spanned<Expr>> {
    let ty = type_expr_parser();

    recursive(|expr: RecursiveDef<Spanned<Expr>>| {
        enum ChainKind {
            Attr(Span, Spanned<String>),
            #[allow(clippy::type_complexity)]
            Call(Vec<(Option<String>, Spanned<Expr>)>, Span),
            Index(Spanned<Expr>),
        }

        fn bin_foldl(
            lhs: Spanned<Expr>,
            (op, rhs): (Spanned<BinaryOp>, Spanned<Expr>),
        ) -> Spanned<Expr> {
            let span = lhs.span().merge(rhs.span());
            Spanned(
                Expr::BinaryOp {
                    left: Box::new(lhs),
                    op,
                    right: Box::new(rhs),
                },
                span,
            )
        }

        // An identifier
        let ident = select! {
            TokenInfo::Ident(name, _) => Expr::Atom(match name.as_str() {
                "true" => Atom::Bool(true),
                "false" => Atom::Bool(false),
                "void" => Atom::Void,
                _ => Atom::Ident(name),
            })
        }
        .map_with_span(Spanned);

        // Parses and consumes the next atom. An atom is the most basic unit of an expression that
        // cannot be broken down into other expressions any further.
        //
        // For example, 1 is an atom, as is "hello" - but 1 + 1 is not, since that can be further
        // broken down into two expressions.
        let atom = filter_map(|span, token| {
            Ok(Spanned(
                Expr::Atom(match token {
                    TokenInfo::IntLiteral(val, info) => Atom::Int(val, info),
                    TokenInfo::FloatLiteral(val) => Atom::Float(val),
                    TokenInfo::StringLiteral(content, flags, inner_span) => {
                        Atom::String(resolve_string(content, flags, inner_span)?)
                    }
                    // FIXME: this needs to raise an error whenever a char literal has more than one character
                    TokenInfo::CharLiteral(content, inner_span) => {
                        let content = resolve_string(content, StringLiteralFlags(0), inner_span)?;
                        Atom::Char(content.chars().next().unwrap())
                    }
                    _ => return Err(Error::expected_input_found(span, None, Some(token))),
                }),
                span,
            ))
        })
        .or(ident)
        .labelled("atom");

        // Intermediate parser to consume comma-separated sequences, e.g. 1, 2, 3
        let comma_separated = expr
            .clone()
            .separated_by(just(TokenInfo::Comma).pad_ws())
            .allow_trailing();

        // Parses expressions that do not have to be orderly disambiguated against
        let unambiguous = choice((
            expr.clone().delimited(Delimiter::Paren),
            comma_separated
                .clone()
                .delimited(Delimiter::Paren)
                .map_with_span(|exprs, span| Spanned(Expr::Tuple(exprs), span))
                .labelled("tuple"),
            comma_separated
                .delimited(Delimiter::Bracket)
                .map_with_span(|exprs, span| Spanned(Expr::Array(exprs), span))
                .labelled("array"),
            atom,
        ))
        .labelled("unambiguous expression")
        .pad_ws()
        .boxed();

        let raw_ident = select!(TokenInfo::Ident(ident, _) => ident);

        let attr = just(TokenInfo::Dot)
            .map_with_span(|_, span: Span| span)
            .then(raw_ident.map_with_span(Spanned))
            .map(|(dot, attr)| ChainKind::Attr(dot, attr));

        let call_args = raw_ident
            .then_ignore(just(TokenInfo::Colon))
            .or_not()
            .pad_ws()
            .then(expr.clone())
            .separated_by(just(TokenInfo::Comma).pad_ws())
            .allow_trailing()
            .delimited(Delimiter::Paren)
            .map_with_span(ChainKind::Call)
            .pad_ws();

        let index = expr
            .clone()
            .delimited_by(
                just(TokenInfo::LeftBracket).pad_ws(),
                just(TokenInfo::RightBracket).pad_ws(),
            )
            .map(ChainKind::Index);

        // A chain of attribute accesses, function calls, and index accesses.
        // These are all left-associative, so a.b.c is parsed as (a.b).c, and a(b)(c) is parsed as
        // ( a(b) )(c).
        let chain = unambiguous
            .clone()
            .map(Ok)
            .then(choice((attr, call_args, index)).repeated())
            .foldl(|lhs: Result<Spanned<Expr>>, kind| {
                let lhs = lhs?;
                match kind {
                    ChainKind::Attr(dot, attr) => {
                        let span = lhs.span().merge(attr.span());
                        Ok(Spanned(
                            Expr::Attr {
                                subject: Box::new(lhs),
                                dot,
                                attr,
                            },
                            span,
                        ))
                    }
                    ChainKind::Call(mut args, span) => {
                        let partition = args
                            .iter()
                            .position(|(name, _)| name.is_some())
                            .unwrap_or(args.len());
                        let kwargs = args.split_off(partition);
                        let span = lhs.span().merge(span);

                        Ok(Spanned(
                            Expr::Call {
                                func: Box::new(lhs),
                                args: args.into_iter().map(|(_, arg)| arg).collect(),
                                kwargs: kwargs
                                    .into_iter()
                                    .map(|(name, arg)| {
                                        Ok((
                                            name.ok_or_else(|| {
                                                Error::unexpected_positional_argument(arg.span())
                                            })?,
                                            arg,
                                        ))
                                    })
                                    .collect::<Result<Vec<_>>>()?,
                            },
                            span,
                        ))
                    }
                    ChainKind::Index(index) => {
                        let span = lhs.span().merge(index.span());
                        Ok(Spanned(
                            Expr::Index {
                                subject: Box::new(lhs),
                                index: Box::new(index),
                            },
                            span,
                        ))
                    }
                }
            })
            .try_map(|e, _span| e)
            .labelled("attribute access, function call, or index access")
            .boxed();

        // Prefix unary operators: -a, +a, !a
        let unary = just(TokenInfo::Minus)
            .to(UnaryOp::Minus)
            .or(just(TokenInfo::Plus).to(UnaryOp::Plus))
            .or(just(TokenInfo::Not).to(UnaryOp::Not))
            .map_with_span(Spanned)
            .pad_ws()
            .repeated()
            .then(chain.clone())
            .foldr(|op, expr| {
                let span = op.span().merge(expr.span());

                Spanned(
                    Expr::UnaryOp {
                        op,
                        expr: Box::new(expr),
                    },
                    span,
                )
            })
            .labelled("unary expression")
            .boxed();

        // Type cast, e.g. a::b
        let cast = unary
            .clone()
            .then(
                just([TokenInfo::Colon, TokenInfo::Colon])
                    .ignore_then(just(TokenInfo::Colon))
                    .pad_ws()
                    .ignore_then(ty.clone())
                    .repeated(),
            )
            .foldl(|target, ty| {
                let span = target.span().merge(ty.span());

                Spanned(
                    Expr::Cast {
                        expr: Box::new(target),
                        ty,
                    },
                    span,
                )
            })
            .labelled("type cast")
            .boxed();

        // Power operator: a ** b
        // Note that this is right-associative, so a ** b ** c is a ** (b ** c)
        let pow = cast
            .clone()
            .then(
                just([TokenInfo::Asterisk, TokenInfo::Asterisk])
                    .to(BinaryOp::Pow)
                    .map_with_span(Spanned)
                    .map(Some)
                    .pad_ws()
                    .then(cast)
                    .repeated(),
            )
            .map(|(head, tail)| {
                tail.into_iter()
                    .rev()
                    .chain(std::iter::once((None, head)))
                    .reduce(|(rop, rhs), (op, lhs)| {
                        let span = lhs.span().merge(rhs.span());
                        (
                            op,
                            Spanned(
                                Expr::BinaryOp {
                                    left: Box::new(lhs),
                                    op: rop.unwrap(),
                                    right: Box::new(rhs),
                                },
                                span,
                            ),
                        )
                    })
                    .unwrap()
                    .1
            })
            .labelled("pow")
            .boxed();

        // Product operators: a * b, a / b, a % b
        let prod = pow
            .clone()
            .then(
                just(TokenInfo::Asterisk)
                    .to(BinaryOp::Mul)
                    .or(just(TokenInfo::Divide).to(BinaryOp::Div))
                    .or(just(TokenInfo::Modulus).to(BinaryOp::Mod))
                    .map_with_span(Spanned)
                    .pad_ws()
                    .then(pow)
                    .repeated(),
            )
            .foldl(bin_foldl)
            .labelled("product")
            .boxed();

        // Sum operators: a + b, a - b
        let sum = prod
            .clone()
            .then(
                just(TokenInfo::Plus)
                    .to(BinaryOp::Add)
                    .or(just(TokenInfo::Minus).to(BinaryOp::Sub))
                    .pad_ws()
                    .map_with_span(Spanned)
                    .then(prod)
                    .repeated(),
            )
            .foldl(bin_foldl)
            .labelled("sum")
            .boxed();

        macro_rules! compound {
            ($ident1:ident $ident2:ident => $to:expr) => {{
                just(TokenInfo::$ident1)
                    .ignore_then(just(TokenInfo::$ident2))
                    .to($to)
            }};
        }

        // Comparison operators: a == b, a != b, a < b, a > b, a <= b, a >= b
        let cmp = sum
            .clone()
            .then(
                compound!(Equals Equals => BinaryOp::Eq)
                    .or(compound!(Not Equals => BinaryOp::Ne))
                    .or(compound!(Lt Equals => BinaryOp::Le))
                    .or(compound!(Gt Equals => BinaryOp::Ge))
                    .or(just(TokenInfo::Lt).to(BinaryOp::Lt))
                    .or(just(TokenInfo::Gt).to(BinaryOp::Gt))
                    .map_with_span(Spanned)
                    .pad_ws()
                    .then(sum)
                    .repeated(),
            )
            .foldl(bin_foldl)
            .labelled("comparison")
            .boxed();

        // Logical AND: a && b
        let logical_and = cmp
            .clone()
            .then(
                compound!(And And => BinaryOp::LogicalAnd)
                    .map_with_span(Spanned)
                    .pad_ws()
                    .then(cmp)
                    .repeated(),
            )
            .foldl(bin_foldl)
            .labelled("logical and")
            .boxed();

        // Logical OR: a || b
        let logical_or = logical_and
            .clone()
            .then(
                compound!(Or Or => BinaryOp::LogicalOr)
                    .map_with_span(Spanned)
                    .pad_ws()
                    .then(logical_and)
                    .repeated(),
            )
            .foldl(bin_foldl)
            .labelled("logical or")
            .boxed();

        // Bitwise operators: a & b, a | b, a ^ b
        let bitwise = logical_or
            .clone()
            .then(
                just(TokenInfo::And)
                    .to(BinaryOp::BitAnd)
                    .or(just(TokenInfo::Or).to(BinaryOp::BitOr))
                    .or(just(TokenInfo::Caret).to(BinaryOp::BitXor))
                    .map_with_span(Spanned)
                    .pad_ws()
                    .then(logical_or)
                    .repeated(),
            )
            .foldl(bin_foldl)
            .labelled("bitwise")
            .boxed();

        // Inline "ternary" if-statement, e.g. if a then b else c
        //
        // We don't have to worry about else-if since
        //     if a then b else if c then d else e
        // ...is parsed as
        //     if a then b else (if c then d else e)
        // ...which works exactly the same.
        let ternary_if = kw!("if")
            .ignore_then(expr.clone())
            .then_ignore(kw!("then"))
            .then(expr.clone())
            .then_ignore(kw!("else"))
            .then(expr.clone())
            .map_with_span(|((cond, then), els), span| {
                Spanned(
                    Expr::If {
                        label: None,
                        cond: Box::new(cond),
                        body: expr_as_block(then),
                        else_if_bodies: Vec::new(),
                        else_body: Some(expr_as_block(els)),
                        ternary: true,
                    },
                    span,
                )
            })
            .pad_ws()
            .boxed();

        // Any expression that must be disambiguated against
        let ambiguous_expr = choice((brace_ending_expr(expr.clone(), body), ternary_if, bitwise));

        macro_rules! op {
            ($($token:ident)+ => $to:ident) => {{
                just([$(TokenInfo::$token),+]).to(AssignmentOperator::$to)
            }};
        }

        let assign_op = choice((
            // Three-character operators
            op!(Lt Lt Equals => ShlAssign),
            op!(Gt Gt Equals => ShrAssign),
            op!(Asterisk Asterisk Equals => PowAssign),
            op!(Or Or Equals => LogicalOrAssign),
            op!(And And Equals => LogicalAndAssign),
            // Two-character operators
            op!(Plus Equals => AddAssign),
            op!(Minus Equals => SubAssign),
            op!(Asterisk Equals => MulAssign),
            op!(Divide Equals => DivAssign),
            op!(Modulus Equals => ModAssign),
            op!(Or Equals => BitOrAssign),
            op!(And Equals => BitAndAssign),
            op!(Caret Equals => BitXorAssign),
            // Ensure that no equals sign is present after the oeprator to remove ambiguity
            // with the == operator.
            op!(Equals => Assign).then_ignore(just(TokenInfo::Equals).not().rewind()),
        ))
        .map_with_span(Spanned)
        .pad_ws();

        // Assignment target
        let assign_target = choice((
            // ambiguous_expr.clone().try_map(|e, _| {
            //     e.try_map(|e| match e {
            //         Expr::Attr { subject, attr, .. } => {
            //             Ok(AssignmentTarget::Attr { subject, attr })
            //         }
            //         Expr::Index { subject, index } => {
            //             Ok(AssignmentTarget::Index { subject, index })
            //         }
            //         _ => Err(Error::default()),
            //     })
            // }),
            pat_parser().try_map(|pat, _| {
                pat.value().assert_immutable_bindings()?;
                Ok(pat.map(AssignmentTarget::Pattern))
            }),
            just(TokenInfo::And)
                .then_ws()
                .ignore_then(expr)
                .map_with_span(|expr, span| {
                    Spanned(AssignmentTarget::Pointer(Box::new(expr)), span)
                }),
        ));

        // Assignment expressions, i.e. a = b, a += b
        assign_target
            .then(assign_op)
            .repeated()
            .then(ambiguous_expr)
            .foldr(|(target, op), expr| {
                let span = target.span().merge(expr.span());
                Spanned(
                    Expr::Assign {
                        target,
                        op,
                        value: Box::new(expr),
                    },
                    span,
                )
            })
            .pad_ws()
            .labelled("assignment")
            .boxed()
    })
}

/// Parses a token stream into an AST.
#[must_use = "parser will only parse if you call its provided methods"]
pub struct Parser<'a> {
    tokens: ChumskyTokenStreamer<'a>,
    eof: Span,
}

impl<'a> Parser<'a> {
    /// Creates a new parser over the provided source provider.
    pub fn from_provider(provider: &'a Provider<'a>) -> Self {
        Self {
            tokens: ChumskyTokenStreamer(TokenReader::new(provider)),
            eof: provider.eof(),
        }
    }

    #[inline]
    fn stream(&mut self) -> Stream<TokenInfo, Span, &mut ChumskyTokenStreamer<'a>> {
        Stream::from_iter(self.eof, &mut self.tokens)
    }

    /// Consumes the next expression in the token stream.
    pub fn next_expr(&mut self) -> StdResult<Spanned<Expr>, Vec<Error>> {
        let body_parser = body_parser();
        expr_parser(body_parser).parse(self.stream())
    }

    /// Consumes the entire token tree as an expression.
    pub fn consume_expr_until_end(&mut self) -> StdResult<Spanned<Expr>, Vec<Error>> {
        let body_parser = body_parser();
        expr_parser(body_parser)
            .then_ignore(end())
            .parse(self.stream())
    }

    /// Consumes the entire token tree as a body.
    pub fn consume_body_until_end(&mut self) -> StdResult<Vec<Spanned<Node>>, Vec<Error>> {
        body_parser().then_ignore(end()).parse(self.stream())
    }
}
