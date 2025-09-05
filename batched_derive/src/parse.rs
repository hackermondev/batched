use std::collections::BTreeMap;

use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::{
    FnArg, GenericArgument, ItemFn, Meta, Pat, PathArguments, ReturnType, Token, Type,
    parse::Parser, punctuated::Punctuated,
};

use crate::utils::expr_to_u64;

#[derive(Debug)]
pub struct Function {
    pub macros: Vec<TokenStream>,
    pub identifier: String,
    pub visibility: TokenStream,
    pub inner: TokenStream,
    pub batched_arg: TokenStream,
    pub batched_arg_name: String,
    pub batched_arg_type: TokenStream,
    pub returned: FunctionResult,
}

#[derive(Debug)]
pub struct FunctionResult {
    pub result_type: FunctionResultType,
    pub tokens: TokenStream,
}

#[derive(Debug)]
pub enum FunctionResultType {
    Raw(TokenStream),
    VectorRaw(TokenStream),
    Result(Box<FunctionResult>, TokenStream, Option<TokenStream>)
}

fn inner_shared_error(_type: &Type) -> Option<TokenStream> {
    let type_path = match _type {
        Type::Path(path) => path,
        _ => unimplemented!(),
    };
    let path = type_path.path.segments.last().unwrap();
    if path.ident != "SharedError" {
        return None;
    }

    match &path.arguments {
        PathArguments::AngleBracketed(path_args) => {
            Some(path_args.args.clone().into_token_stream())
        }
        _ => unimplemented!(),
    }
}

impl Function {
    pub fn parse(tokens: TokenStream) -> Self {
        let function: ItemFn = syn::parse2(tokens).expect("invalid function");

        let macros = function
            .attrs
            .into_iter()
            .map(|attr| attr.into_token_stream())
            .collect();

        let visibility = function.vis.into_token_stream();
        let identifier = function.sig.ident.to_string();
        let args = function.sig.inputs;
        let inner = function.block.to_token_stream();

        fn parsed_returned(_type: &Type) -> FunctionResult {
            let tokens = _type.clone().into_token_stream();
            let result_type = match _type {
                Type::Path(type_path) => {
                    let path = type_path.path.segments.first().unwrap();

                    if path.ident == "Vec" {
                        let inner = match &path.arguments {
                            PathArguments::AngleBracketed(b) => b,
                            _ => unimplemented!(),
                        };
                        let inner = inner.args.to_token_stream();
                        FunctionResultType::VectorRaw(inner)
                    } else if path.ident == "Result" {
                        let inner = match &path.arguments {
                            PathArguments::AngleBracketed(b) => b,
                            _ => unimplemented!(),
                        };

                        let output = inner.args.get(0).unwrap();
                        let error = inner.args.get(1).unwrap();
                        let error = match error {
                            GenericArgument::Type(error) => error,
                            _ => unimplemented!(),
                        };

                        let output = match output {
                            GenericArgument::Type(_type) => parsed_returned(_type),
                            _ => unimplemented!(),
                        };
                        let inner_shared_error = inner_shared_error(error);
                        let error = error.into_token_stream();

                        FunctionResultType::Result(Box::new(output), error, inner_shared_error)
                    } else {
                        FunctionResultType::Raw(_type.into_token_stream())
                    }
                }
                _ => FunctionResultType::Raw(_type.into_token_stream()),
            };

            FunctionResult {
                tokens,
                result_type,
            }
        }
        let returned = match function.sig.output {
            ReturnType::Default => FunctionResult {
                tokens: syn::parse_str("()").unwrap(),
                result_type: FunctionResultType::Raw(syn::parse_str("()").unwrap()),
            },
            ReturnType::Type(_, _type) => parsed_returned(&_type),
        };

        let mut batched_arg: Option<TokenStream> = None;
        let mut batched_arg_name: Option<String> = None;
        let mut batched_arg_type: Option<TokenStream> = None;

        for arg in args {
            if let FnArg::Receiver(_) = arg {
                panic!("self reference functions are not supported")
            } else if let FnArg::Typed(arg) = arg {
                if batched_arg_type.is_some() {
                    panic!("function may only contain a single argument")
                }

                batched_arg = Some(arg.to_token_stream());
                batched_arg_name = match &*arg.pat {
                    Pat::Ident(pat_ident) => Some(pat_ident.ident.to_string()),
                    _ => panic!("unsupport argument name"),
                };

                if let syn::Type::Path(type_path) = &*arg.ty {
                    let segment = type_path.path.segments.last().unwrap();
                    if segment.ident == "Vec" {
                        let vec_type = match &segment.arguments {
                            PathArguments::AngleBracketed(a) => a,
                            _ => unreachable!(),
                        };
                        let vec_type = vec_type.args.first().unwrap();
                        batched_arg_type = Some(vec_type.to_token_stream());
                        continue;
                    }
                }

                panic!("function argument must be Vec<T>")
            }
        }

        if batched_arg_type.is_none() {
            panic!("function must contain a single argument")
        }

        let batched_arg = batched_arg.unwrap();
        let batched_arg_name = batched_arg_name.unwrap();
        let batched_arg_type = batched_arg_type.unwrap();

        Self {
            macros,
            identifier,
            visibility,
            batched_arg,
            batched_arg_name,
            batched_arg_type,
            returned,
            inner,
        }
    }
}

#[derive(Debug)]
pub struct Attributes {
    pub limit: usize,
    pub concurrent_limit: Option<usize>,
    pub asynchronous: bool,
    pub default_window: u64,
    pub windows: BTreeMap<u64, u64>,
}

impl Attributes {
    pub fn parse(tokens: TokenStream) -> Self {
        let mut limit: Option<usize> = None;
        let mut concurrent_limit: Option<usize> = None;
        let mut asynchronous = false;
        let mut default_window: Option<u64> = None;
        let mut windows = BTreeMap::new();

        static WINDOW_ATTR: &str = "window";
        static LIMIT_ATTR: &str = "limit";
        static CONCURRENT_LIMIT_ATTR: &str = "concurrent";
        static ASYNCHRONOUS_ATTR: &str = "asynchronous";

        let parser = Punctuated::<Meta, Token![,]>::parse_separated_nonempty;
        let attributes = parser.parse(tokens.into()).unwrap();
        let attributes = attributes.into_iter().collect::<Vec<Meta>>();

        for attr in &attributes {
            let path = attr.path();
            if path.is_ident(LIMIT_ATTR) {
                let value = match attr {
                    Meta::NameValue(attr) => &attr.value,
                    _ => unimplemented!(),
                };

                limit = expr_to_u64(value).map(|u| u as usize);
            } else if path.is_ident(CONCURRENT_LIMIT_ATTR) {
                let value = match attr {
                    Meta::NameValue(attr) => &attr.value,
                    _ => unimplemented!(),
                };

                concurrent_limit = expr_to_u64(value).map(|u| u as usize);
            } else if path.is_ident(ASYNCHRONOUS_ATTR) {
                asynchronous = true;
            } else if path.is_ident(WINDOW_ATTR) {
                let value = match attr {
                    Meta::NameValue(attr) => &attr.value,
                    _ => unimplemented!(),
                };

                let window_duration_ms = expr_to_u64(value);
                default_window = window_duration_ms;
            } else if let Some(ident) = path.get_ident().map(|i| i.to_string())
                && ident.starts_with(WINDOW_ATTR)
            {
                let value = match attr {
                    Meta::NameValue(attr) => &attr.value,
                    _ => unimplemented!(),
                };

                let call_size = ident.replace(WINDOW_ATTR, "");
                let call_size = call_size.parse::<u64>().unwrap();
                let call_window = expr_to_u64(value).expect("expected u64");

                let unsorted = windows
                    .iter()
                    .find(|(_call_size, _)| **_call_size > call_size);
                if unsorted.is_some() {
                    panic!("dynamic window call size must be sorted")
                }

                windows.insert(call_size, call_window);
            }
        }

        let default_window = default_window.expect("expected required attribute: window");
        let limit = limit.expect("expected required attribute: limit");

        Self {
            limit,
            concurrent_limit,
            asynchronous,
            default_window,
            windows,
        }
    }
}