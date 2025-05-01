use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::{
    FnArg, ItemFn, Meta, Pat, PathArguments, ReturnType, Token, parse::Parser,
    punctuated::Punctuated,
};

use crate::utils::expr_to_u64;

#[derive(Debug)]
pub struct Function {
    pub identifier: String,
    pub visibility: TokenStream,
    pub batched_arg: TokenStream,
    pub batched_arg_name: String,
    pub batched_arg_type: TokenStream,
    pub return_value: TokenStream,
    pub inner: TokenStream,
}

impl Function {
    pub fn parse(tokens: TokenStream) -> Self {
        let function: ItemFn = syn::parse(tokens.into()).expect("invalid function");

        let visibility = function.vis.into_token_stream();
        let identifier = function.sig.ident.to_string();
        let args = function.sig.inputs;
        let inner = function.block.to_token_stream();

        let return_value = match function.sig.output {
            ReturnType::Default => syn::parse_str("()").unwrap(),
            ReturnType::Type(_, _type) => _type.into_token_stream(),
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
            identifier,
            visibility,
            batched_arg,
            batched_arg_name,
            batched_arg_type,
            return_value,
            inner,
        }
    }
}

#[derive(Debug)]
pub struct Attributes {
    pub window: u64,
    pub limit: usize,
    pub concurrent_limit: Option<usize>,
    pub wrap_in_arc: bool,
}

impl Attributes {
    pub fn parse(tokens: TokenStream) -> Self {
        let mut window: Option<u64> = None;
        let mut limit: Option<usize> = None;
        let mut concurrent_limit: Option<usize> = None;
        let mut wrap_in_arc = false;

        static WINDOW_ATTR: &str = "window";
        static LIMIT_ATTR: &str = "limit";
        static CONCURRENT_LIMIT_ATTR: &str = "concurrent";
        static WRAP_ARC_ATTR: &str = "boxed";

        let parser = Punctuated::<Meta, Token![,]>::parse_separated_nonempty;
        let attributes = parser.parse(tokens.into()).unwrap();
        let attributes = attributes.into_iter().collect::<Vec<Meta>>();

        for attr in &attributes {
            let path = attr.path();
            if path.is_ident(WINDOW_ATTR) {
                let value = match attr {
                    Meta::NameValue(attr) => &attr.value,
                    _ => unimplemented!(),
                };

                let window_duration_ms = expr_to_u64(value);
                window = window_duration_ms;
            } else if path.is_ident(LIMIT_ATTR) {
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
            } else if path.is_ident(WRAP_ARC_ATTR) {
                wrap_in_arc = true;
            }
        }

        let window = window.expect("expected required attribute: window");
        let limit = limit.expect("expected required attribute: limit");
        Self {
            window,
            limit,
            concurrent_limit,
            wrap_in_arc,
        }
    }
}
