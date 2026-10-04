extern crate proc_macro;

mod builder;
mod parse;
mod utils;

use builder::build_code;
use parse::{Attributes, Function};
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn batched(attributes: TokenStream, item: TokenStream) -> TokenStream {
    let attributes = Attributes::parse(attributes.into());
    let function = Function::parse(item.into());
    let _identifier = function.identifier.clone();

    let result = build_code(function, attributes).into();
    if option_env!("BATCHED_DEBUG_OUTPUT").is_some() {
        println!("{}: {result}", _identifier);
    }
    result
}
