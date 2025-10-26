use std::usize;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::parse::{Attributes, Function, FunctionResultType};

struct Identifiers {
    public_interface: Ident,
    public_interface_multiple: Ident,
    inner_batched: Ident,
    executor_producer_channel: Ident,
    executor_background_fn: Ident,
}

fn build_identifiers(call_function: &Function) -> Identifiers {
    let id = &call_function.identifier;

    let public_interface = format_ident!("{id}");
    let public_interface_multiple = format_ident!("{id}_multiple");
    let inner_batched = format_ident!("{id}__batched");

    let executor_producer_channel = format_ident!("BATCHED_{}", id.to_uppercase());
    let executor_background_fn = format_ident!("spawn_executor_{id}");

    Identifiers {
        public_interface,
        public_interface_multiple,
        inner_batched,
        executor_producer_channel,
        executor_background_fn,
    }
}

pub fn build_code(function: Function, options: Attributes) -> TokenStream {
    let identifiers = build_identifiers(&function);
    let executor = build_executor(&identifiers, &function, &options);
    let public_interface = build_public_interface(&identifiers, &function, &options);

    quote! {
        #executor
        #public_interface
    }
}

fn build_public_interface(
    identifiers: &Identifiers,
    call_function: &Function,
    options: &Attributes,
) -> TokenStream {
    let macros = &call_function.macros;
    let visibility = &call_function.visibility;
    let arg = &call_function.batched_arg;
    let arg_name: TokenStream = syn::parse_str(&call_function.batched_arg_name).unwrap();
    let arg_type = &call_function.batched_arg_type;
    let inner_body = &call_function.inner;
    let returned = &call_function.returned.tokens;

    let (is_result, is_vec) = function_flags(&call_function);
    let asynchronous = options.asynchronous;

    let return_type = match &call_function.returned.result_type {
        FunctionResultType::Raw(token) => token.clone(),
        FunctionResultType::VectorRaw(token) => token.clone(),
        FunctionResultType::Result(output, error, _) => {
            let tokens = &output.tokens;
            match &output.result_type {
                FunctionResultType::VectorRaw(token) => quote! { Result<#token, #error> },
                _ => quote! { Result<#tokens, #error> },
            }
        }
    };
    let return_result = if is_result {
        if is_vec {
            quote! {
                let mut result = result?;
                let result = result.remove(0);
                Ok(result)
            }
        } else {
            quote! {
                let result = result?;
                Ok(result)
            }
        }
    } else if is_vec {
        quote! {
            let result = result.remove(0);
            result
        }
    } else {
        quote! {
            result
        }
    };

    let return_type_multiple = match &call_function.returned.result_type {
        FunctionResultType::Raw(token) => token.clone(),
        FunctionResultType::VectorRaw(token) => quote! { Vec<#token> },
        FunctionResultType::Result(output, error, _) => {
            let tokens = &output.tokens;
            match &output.result_type {
                FunctionResultType::VectorRaw(token) => quote! { Result<Vec<#token>, #error> },
                _ => quote! { Result<#tokens, #error> },
            }
        }
    };
    let return_result_multiple = if is_result {
        quote! {
            let result = result?;
            Ok(result)
        }
    } else {
        quote! { result }
    };

    let cast_result_error = match &call_function.returned.result_type {
        FunctionResultType::Raw(_) => None,
        FunctionResultType::VectorRaw(_) => None,
        FunctionResultType::Result(_, _, inner_shared_error) => inner_shared_error.as_ref().map(|inner_shared_error| quote! {
                let result = result.map_err(|e: #inner_shared_error| e.into());
            }),
    };

    let executor_producer_channel = &identifiers.executor_producer_channel;
    let executor_background_fn = &identifiers.executor_background_fn;
    let inner_batched = &identifiers.inner_batched;
    let public_interface = &identifiers.public_interface;
    let public_interface_multiple = &identifiers.public_interface_multiple;

    #[cfg(feature = "tracing_span")]
    let tracing_span = quote! { #[tracing::instrument(skip_all)] };
    #[cfg(not(feature = "tracing_span"))]
    let tracing_span = quote! {};

    let inner_batched = quote! {
        #(#macros)*
        async fn #inner_batched(#arg) -> #returned {
            let result = async { #inner_body };
            let result = result.await;
            #cast_result_error
            result
        }
    };

    if asynchronous {
        quote! {
            #inner_batched

            #tracing_span
            #visibility async fn #public_interface(#arg_name: #arg_type) {
                #public_interface_multiple(vec![#arg_name]).await;
            }

            #tracing_span
            #visibility async fn #public_interface_multiple(#arg_name: Vec<#arg_type>) {
                let channel = &#executor_producer_channel;
                let channel = channel.get_or_init(async || { #executor_background_fn().await }).await;

                let span = ::batched::tracing::Span::current();
                channel.send((#arg_name, span, None)).await
                    .expect("batched function panicked (send)");
            }
        }
    } else {
        quote! {
            #inner_batched

            #tracing_span
            #visibility async fn #public_interface(#arg_name: #arg_type) -> #return_type {
                let mut result = #public_interface_multiple(vec![#arg_name]).await;
                #return_result
            }

            #tracing_span
            #visibility async fn #public_interface_multiple(#arg_name: Vec<#arg_type>) -> #return_type_multiple {
                let channel = &#executor_producer_channel;
                let channel = channel.get_or_init(async || { #executor_background_fn().await }).await;

                let (response_channel_sender, mut response_channel_recv) = ::tokio::sync::mpsc::channel(1);
                let span = ::batched::tracing::Span::current();
                channel.send((#arg_name, span, Some(response_channel_sender))).await
                    .expect("batched function panicked (send)");

                let result = response_channel_recv.recv().await
                    .expect("batched function panicked (recv)");
                #return_result_multiple
            }
        }
    }
    
}

fn build_executor(
    identifiers: &Identifiers,
    call_function: &Function,
    options: &Attributes,
) -> TokenStream {
    const SEMAPHORE_MAX_PERMITS: usize = 2305843009213693951;

    let capacity = options.limit;
    let concurrent_limit = options.concurrent_limit.unwrap_or(SEMAPHORE_MAX_PERMITS);
    let default_window = options.default_window;
    let asynchronous = options.asynchronous;
    
    let windows = options.windows.iter();
    let windows = windows.map(|(call_size, call_window)| {
        quote! { windows.insert(#call_size, #call_window); }
    });
    let windows = quote! {
        let mut windows = ::std::collections::BTreeMap::<u64, u64>::new();
        #(#windows)*
    };

    let arg_type = &call_function.batched_arg_type;
    let returned_type_plural = match &call_function.returned.result_type {
        FunctionResultType::Raw(token) => token.clone(),
        FunctionResultType::VectorRaw(token) => quote! { Vec<#token> },
        FunctionResultType::Result(output, error, _) => {
            let tokens = &output.tokens;
            match &output.result_type {
                FunctionResultType::VectorRaw(token) => quote! { Result<Vec<#token>, #error> },
                _ => quote! { Result<#tokens, #error> },
            }
        }
    };

    let (is_result, is_vec) = function_flags(call_function);

    let handle_result = if is_result {
        if is_vec {
            quote! {
                let result = result.as_mut().map(|r| r.drain(..count).collect()).map_err(|e| e.clone());
            }
        } else {
            quote! {
                let result = result.clone();
            }
        }
    } else if is_vec {
        quote! {
            let result = result.drain(..count).collect();
        }
    } else {
        quote! {
            let result = result.clone();
        }
    };

    let channel_type = quote! { (Vec<#arg_type>, ::batched::tracing::Span, Option<::tokio::sync::mpsc::Sender<#returned_type_plural>>) };
    let propagate_result = if asynchronous { quote! {} } else {
        quote! {
            for (channel, count) in channels {
                #handle_result
                if let Some(channel) = channel {
                    let _ = channel.try_send(result);
                }
            }
        }
    };

    let inner_batched = &identifiers.inner_batched;
    let batched_span_name = inner_batched.to_string();
    let executor_producer_channel = &identifiers.executor_producer_channel;
    let executor_background_fn = &identifiers.executor_background_fn;

    quote! {
        static #executor_producer_channel:
            ::tokio::sync::OnceCell<::tokio::sync::mpsc::Sender<#channel_type>> = ::tokio::sync::OnceCell::const_new();

        async fn #executor_background_fn() -> ::tokio::sync::mpsc::Sender<#channel_type> {
            let capacity = #capacity;
            let default_window = #default_window;
            #windows


            let (sender, mut receiver) = tokio::sync::mpsc::channel(capacity);
            tokio::task::spawn(async move {
                let semaphore = ::std::sync::Arc::new(::tokio::sync::Semaphore::new(#concurrent_limit));

                loop {
                    let mut data_buffer = Vec::new();
                    let mut return_channels: Vec<(Option<::tokio::sync::mpsc::Sender<#returned_type_plural>>, usize)> = vec![];
                    let mut waiting_spans: Vec<::batched::tracing::Span> = vec![];

                    let window_start = ::std::time::Instant::now();

                    loop {
                        let window = windows.iter()
                            .find(|(max_calls, _)|  **max_calls >= data_buffer.len() as u64)
                            .map(|(_, window)| window);
                        let window = window.unwrap_or(&default_window);
                        let window = ::std::time::Duration::from_millis(*window as u64);

                        let window_end = window_start + window;
                        let remaining_duration = window_end.duration_since(std::time::Instant::now());

                        tokio::select! {
                            event = receiver.recv() => {
                                if event.is_none() {
                                    return;
                                }

                                let event: #channel_type = event.unwrap();
                                let (mut data, span, channel) = event;

                                return_channels.push((channel, data.len()));
                                waiting_spans.push(span);
                                data_buffer.append(&mut data);

                                if data_buffer.len() >= capacity {
                                    break;
                                }
                            }

                            _ = ::tokio::time::sleep(remaining_duration) => {
                                break;
                            }
                        }
                    }

                    if return_channels.is_empty() {
                        continue;
                    }

                    let mut data = vec![];
                    let mut spans = vec![];
                    let mut channels = vec![];

                    std::mem::swap(&mut data, &mut data_buffer);
                    std::mem::swap(&mut spans, &mut waiting_spans);
                    std::mem::swap(&mut channels, &mut return_channels);

                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    tokio::task::spawn(async move {
                        let _permit = permit;
                        let batched_span = ::batched::tracing::info_span!(#batched_span_name, count = data.len());
                        for mut span in spans {
                            ::batched::tracing::TracingSpan::link_span(&mut span, &batched_span);
                        }

                        let future = #inner_batched(data);
                        let mut result = ::batched::tracing::Instrument::instrument(future, batched_span).await;
                        #propagate_result
                    });
                }
            });

            sender
        }
    }
}

// TODO: Move this to [`Function`] parser
fn function_flags(function: &Function) -> (bool, bool) {
    let mut is_result = false;
    let mut is_vec = false;

    match &function.returned.result_type {
        FunctionResultType::Result(result, _, _) => {
            is_result = true;
            if let FunctionResultType::VectorRaw(_) = result.result_type {
                is_vec = true
            };
        }
        FunctionResultType::VectorRaw(_) => is_vec = true,
        _ => {}
    };
    (is_result, is_vec)
}
