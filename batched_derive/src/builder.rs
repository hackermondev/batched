use std::usize;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::types::{Attributes, Function};

static MAX_SEMAPHORE_PERMITS: usize = 2305843009213693951;

pub fn build_code(call_function: Function, options: Attributes) -> TokenStream {
    let name = call_function.identifier.replace("_batched", "");
    let visibility = call_function.visibility;
    let arg = call_function.batched_arg;
    let arg_name = call_function.batched_arg_name;
    let arg_type = call_function.batched_arg_type;
    let returned = call_function.return_value;
    let inner_body = call_function.inner;

    let capacity = options.limit;
    let window = options.window;
    let concurrent_limit = options.concurrent_limit.unwrap_or(MAX_SEMAPHORE_PERMITS);

    let batched_producer_channel =
        format_ident!("BATCHED_{}_PRODUCER_CHANNEL", name.to_uppercase());
    let __spawn_background_batch = format_ident!("__spawn_background_{name}_batched");
    let batched = format_ident!("__{name}_batched");

    let name = syn::parse_str::<TokenStream>(&name).unwrap();
    let arg_name = syn::parse_str::<TokenStream>(&arg_name).unwrap();
    let fnname_multiple = format_ident!("{name}_multiple");
    let returned_arc = if options.wrap_in_arc {
        syn::parse_str(&format!("::std::sync::Arc<{returned}>")).unwrap()
    } else {
        returned.clone()
    };
    let wrap_in_arc = if options.wrap_in_arc {
        Some(syn::parse_str::<TokenStream>("let result = ::std::sync::Arc::new(result);").unwrap())
    } else {
        None
    };

    quote! {
        static #batched_producer_channel:
            ::tokio::sync::OnceCell<::tokio::sync::mpsc::Sender<(Vec<#arg_type>, ::tokio::sync::mpsc::Sender<#returned_arc>)>> = ::tokio::sync::OnceCell::const_new();

        async fn #__spawn_background_batch() -> ::tokio::sync::mpsc::Sender<(Vec<#arg_type>, ::tokio::sync::mpsc::Sender<#returned_arc>)> {
            let capacity = #capacity;
            let window = tokio::time::Duration::from_millis(#window);

            let (sender, mut receiver) = tokio::sync::mpsc::channel(capacity);
            tokio::task::spawn(async move {
                let mut buffer = Vec::with_capacity(capacity);
                let mut channels: Vec<::tokio::sync::mpsc::Sender<#returned_arc>> = vec![];
                let semaphore = ::std::sync::Arc::new(::tokio::sync::Semaphore::new(#concurrent_limit));

                loop {
                    let mut timer = tokio::time::interval(window);

                    loop {
                        tokio::select! {
                            event = receiver.recv() => {
                                if event.is_none() {
                                    return;
                                }

                                if buffer.is_empty() {
                                    timer.reset();
                                }

                                let (mut calls, channel) = event.unwrap();
                                buffer.append(&mut calls);
                                channels.push(channel);
                                if buffer.len() >= capacity {
                                    break;
                                }
                            }

                            _ = async {
                                if buffer.is_empty() {
                                    std::future::pending().await
                                } else {
                                    timer.tick().await
                                }
                            } => {
                                break;
                            }
                        }
                    }

                    let mut calls = vec![];
                    let mut return_channels = vec![];

                    std::mem::swap(&mut calls, &mut buffer);
                    std::mem::swap(&mut return_channels, &mut channels);
                    if calls.is_empty() && return_channels.is_empty() {
                        continue
                    }

                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    tokio::task::spawn(async move {
                        let _permit = permit;

                        let result = #batched(calls).await;
                        #wrap_in_arc
                        for channel in return_channels {
                            let _ = channel.try_send(result.clone());
                        }
                    });

                    buffer.reserve(capacity);
                }
            });

            sender
        }

        async fn #batched(#arg) -> #returned {
            #inner_body
        }

        #visibility async fn #name(#arg_name: #arg_type) -> #returned_arc {
            #fnname_multiple(vec![#arg_name]).await
        }

        #visibility async fn #fnname_multiple(#arg_name: Vec<#arg_type>) -> #returned_arc {
            let channel = &#batched_producer_channel;
            let channel = channel.get_or_init(async || { #__spawn_background_batch().await }).await;

            let (response_channel_sender, mut response_channel_recv) = tokio::sync::mpsc::channel(1);
            channel.send((#arg_name, response_channel_sender)).await
                .expect("batched background thread is gone");

            let result = response_channel_recv.recv().await.expect("task panicked");
            result
        }
    }
}
