extern crate proc_macro;

mod builder;
mod types;
mod utils;

use builder::build_code;
use proc_macro::TokenStream;
use types::{Attributes, Function};

#[proc_macro_attribute]
pub fn batched(attributes: TokenStream, item: TokenStream) -> TokenStream {
    let attributes = Attributes::parse(attributes.into());
    let function = Function::parse(item.into());

    let result = build_code(function, attributes).into();
    #[cfg(test)]
    println!("{result}");

    result
}
