use std::usize;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse_str;

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

    let name = parse_str::<TokenStream>(&name).unwrap();
    let arg_name = parse_str::<TokenStream>(&arg_name).unwrap();
    let fnname_multiple = format_ident!("{name}_multiple");

    let has_iterator_value = options.returned_iterator.is_some();
    let returned_type = if let Some(iterator_value) = options.returned_iterator {
        iterator_value
    } else if options.wrap_in_arc {
        parse_str(&format!("::std::sync::Arc<{returned}>")).unwrap()
    } else {
        returned.clone()
    };
    let returned_type_multiple = if has_iterator_value {
        parse_str(&format!("Vec<{returned_type}>")).unwrap()
    } else {
        returned_type.clone()
    };

    let wrap_in_arc = if options.wrap_in_arc {
        Some(parse_str::<TokenStream>("let result = ::std::sync::Arc::new(result);").unwrap())
    } else {
        None
    };
    let handle_result: TokenStream = if has_iterator_value {
        quote! {
            let result = #batched(calls).await;
            let mut result = result.into_iter();
            for (channel, count) in return_channels {
                let chunk = result.by_ref().take(count).collect();
                let _ = channel.try_send(chunk);
            }
        }
    } else {
        quote! {
            let result = #batched(calls).await;
            #wrap_in_arc
            for (channel, count) in return_channels {
                let _ = channel.try_send(result.clone());
            }
        }
    };

    let singular_batch_function = if has_iterator_value {
        quote! {
            #visibility async fn #name(#arg_name: #arg_type) -> #returned_type {
                let mut vec = #fnname_multiple(vec![#arg_name]).await;
                vec.remove(0)
            }
        }
    } else {
        quote! {
            #visibility async fn #name(#arg_name: #arg_type) -> #returned_type {
                #fnname_multiple(vec![#arg_name]).await
            }
        }
    };

    let channel_type = quote! { (Vec<#arg_type>, ::tokio::sync::mpsc::Sender<#returned_type_multiple>) };
    quote! {
        static #batched_producer_channel:
            ::tokio::sync::OnceCell<::tokio::sync::mpsc::Sender<#channel_type>> = ::tokio::sync::OnceCell::const_new();

        async fn #__spawn_background_batch() -> ::tokio::sync::mpsc::Sender<#channel_type> {
            let capacity = #capacity;
            let window = tokio::time::Duration::from_millis(#window);

            let (sender, mut receiver) = tokio::sync::mpsc::channel(capacity);
            tokio::task::spawn(async move {
                let mut buffer = Vec::with_capacity(capacity);
                let mut channels: Vec<(::tokio::sync::mpsc::Sender<#returned_type_multiple>, usize)> = vec![];
                let semaphore = ::std::sync::Arc::new(::tokio::sync::Semaphore::new(#concurrent_limit));

                loop {
                    let mut timer = tokio::time::interval(window);
                    let mut recieved_first_batch = false;

                    loop {
                        tokio::select! {
                            event = receiver.recv() => {
                                if event.is_none() {
                                    return;
                                }

                                if !recieved_first_batch {
                                    timer.reset();
                                }
                                recieved_first_batch = true;

                                let (mut calls, channel): (Vec<#arg_type>, ::tokio::sync::mpsc::Sender<#returned_type_multiple>) = event.unwrap();
                                channels.push((channel, calls.len()));
                                buffer.append(&mut calls);
                                if buffer.len() >= capacity {
                                    break;
                                }
                            }

                            _ = async {
                                if !recieved_first_batch {
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
                        #handle_result
                    });

                    buffer.reserve(capacity);
                }
            });

            sender
        }

        async fn #batched(#arg) -> #returned #inner_body

        #singular_batch_function 

        #visibility async fn #fnname_multiple(#arg_name: Vec<#arg_type>) -> #returned_type_multiple {
            let channel = &#batched_producer_channel;
            let channel = channel.get_or_init(async || { #__spawn_background_batch().await }).await;

            let (response_channel_sender, mut response_channel_recv) = tokio::sync::mpsc::channel(1);
            channel.send((#arg_name, response_channel_sender)).await
                .expect("batched function panicked");

            let result = response_channel_recv.recv().await
                .expect("batched function panicked");
            result
        }
    }
}
