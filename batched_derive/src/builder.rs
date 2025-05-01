use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::types::{Attributes, Function};

pub fn build_code(call_function: Function, options: Attributes) -> TokenStream {
    let name = call_function.identifier.replace("_batched", "");
    let visibility = call_function.visibility;
    let arg_name = call_function.batched_arg_name;
    let arg_type = call_function.batched_arg_type;
    let returned = call_function.return_value;
    let inner_body = call_function.inner;
    
    let capacity = options.limit;
    let window = options.window;

    let batched_producer_channel = format_ident!("BATCHED_{}_PRODUCER_CHANNEL", name.to_uppercase());
    let __spawn_background_batch = format_ident!("__spawn_background_{name}_batched");
    let batched = format_ident!("__{name}_batched");

    let name = syn::parse_str::<TokenStream>(&name).unwrap();
    let arg_name = syn::parse_str::<TokenStream>(&arg_name).unwrap();
    let fnname_multiple = format_ident!("{name}_multiple");

    quote! {
        static #batched_producer_channel:
            ::tokio::sync::OnceCell<::tokio::sync::mpsc::Sender<(Vec<#arg_type>, ::tokio::sync::mpsc::Sender<#returned>)>> = ::tokio::sync::OnceCell::const_new();

        async fn #__spawn_background_batch() -> ::tokio::sync::mpsc::Sender<(Vec<#arg_type>, ::tokio::sync::mpsc::Sender<#returned>)> {
            let capacity = #capacity;
            let window = tokio::time::Duration::from_millis(#window);

            let (sender, mut receiver) = tokio::sync::mpsc::channel(capacity);
            tokio::task::spawn(async move {
                let mut buffer = Vec::with_capacity(capacity);
                let mut channels: Vec<::tokio::sync::mpsc::Sender<#returned>> = vec![];

                loop {
                    let mut timer = tokio::time::interval(window);
                    timer.tick().await;

                    loop {
                        tokio::select! {
                            event = receiver.recv() => {
                                if event.is_none() {
                                    return;
                                }

                                let (mut calls, channel) = event.unwrap();
                                buffer.append(&mut calls);
                                channels.push(channel);
                                if buffer.len() >= capacity {
                                    break;
                                }
                            }

                            _ = timer.tick() => {
                                break;
                            }
                        }
                    }

                    let mut calls = vec![];
                    std::mem::swap(&mut calls, &mut buffer);
                    if calls.is_empty() {
                        continue
                    }

                    let result = #batched(calls).await;
                    for channel in &channels {
                        let _ = channel.try_send(result.clone());
                    }

                    channels.clear();
                    buffer.reserve(capacity);
                }
            });

            sender
        }

        async fn #batched(#arg_name: Vec<#arg_type>) -> #returned {
            #inner_body
        }

        #visibility async fn #name(call: #arg_type) -> #returned {
            #fnname_multiple(vec![call]).await
        }

        #visibility async fn #fnname_multiple(calls: Vec<#arg_type>) -> #returned {
            let channel = &#batched_producer_channel;
            let channel = channel.get_or_init(async || { #__spawn_background_batch().await }).await;

            let (response_channel_sender, mut response_channel_recv) = tokio::sync::mpsc::channel(1);
            channel.send((calls, response_channel_sender)).await
                .expect("batched background thread is gone");

            let result = response_channel_recv.recv().await.expect("task panicked");
            result
        }
    }
}
