use std::usize;

use inflection::{plural, singular};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::parse::{Attributes, Function, FunctionResultType};

struct Identifiers {
    public_interface: Ident,
    public_interface_arg: Ident,
    public_interface_plural: Ident,
    public_interface_plural_arg: Ident,
    inner_batched: Ident,
    inner_passthrough: Ident,
    executor_batch_data_struct_type: Ident,
    executor_producer_channel: Ident,
    executor_background_fn: Ident,
}

fn build_identifiers(call_function: &Function) -> Identifiers {
    let id = &call_function.identifier;
    let id = if let Some(batched_pos) = id.find("_batched") {
        (&id[..batched_pos]).to_string()
    } else { id.to_string() };
    let id = &id;

    let arg_name = &call_function.batched_arg_name;

    let public_interface = format_ident!("{id}");
    let public_interface_arg = format_ident!("{}", singular::<_, String>(arg_name));

    let public_interface_plural = format_ident!("{}", plural::<_, String>(id));
    let public_interface_plural_arg = format_ident!("{}", plural::<_, String>(arg_name));

    let inner_batched = format_ident!("{id}__batched");
    let inner_passthrough = format_ident!("{id}__passthrough");

    let executor_batch_data_struct_type = format_ident!("{id}BatchedData");
    let executor_producer_channel = format_ident!("BATCHED_{}", id.to_uppercase());
    let executor_background_fn = format_ident!("spawn_executor_{id}");

    Identifiers {
        public_interface,
        public_interface_arg,
        public_interface_plural,
        public_interface_plural_arg,
        inner_batched,
        inner_passthrough,
        executor_batch_data_struct_type,
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

fn build_executor(
    identifiers: &Identifiers,
    call_function: &Function,
    options: &Attributes,
) -> TokenStream {
    const SEMAPHORE_MAX_PERMITS: usize = 2305843009213693951;

    let capacity = options.limit.unwrap_or(usize::MAX);
    let concurrent_limit = options.concurrent_limit.unwrap_or(SEMAPHORE_MAX_PERMITS);
    let window = options.window;
    let asynchronous = options.asynchronous;
    let partition_resolver = match &options.partition_resolver {
        Some(partition_resolver) => {
            let data_type = &call_function.batched_arg_type;
            let partition_resolver_call = match partition_resolver {
                crate::parse::PartitionResolver::Function(expr) => quote! {
                    let partition_resolver = #expr;
                    let partition = partition_resolver(x);
                },
                crate::parse::PartitionResolver::AsyncFunction(expr) => quote! {
                    let partition_resolver = #expr;
                    let partition = partition_resolver(x).await;
                },
            };

            quote! {
                let partition_resolver = async |x: &#data_type| {
                    use std::hash::{Hash, Hasher};

                    #partition_resolver_call

                    let mut hasher = ::std::hash::DefaultHasher::new();
                    partition.hash(&mut hasher);

                    let partition_hash = hasher.finish();
                    partition_hash
                };
                let partition_resolver = Some(partition_resolver);
            }
        }
        None => {
            let data_type = &call_function.batched_arg_type;
            quote! {
                let partition_resolver:
                    Option<Box<dyn Fn(&#data_type) -> std::pin::Pin<Box<dyn Future<Output = u64> + Send + Sync>> + Send + Sync>> = None;
            }
        }
    };

    let arg_type = &call_function.batched_arg_type;
    let returned_type_plural = match &call_function.returned.result_type {
        FunctionResultType::Raw(token) => quote! { (#token, Vec<usize>) },
        FunctionResultType::VectorRaw(token) => quote! { (Vec<#token>, Vec<usize>) },
        FunctionResultType::Result(output, error, _) => {
            let tokens = &output.tokens;
            match &output.result_type {
                FunctionResultType::VectorRaw(token) => {
                    quote! { (Result<Vec<#token>, #error>, Vec<usize>) }
                }
                _ => quote! { (Result<#tokens, #error>, Vec<usize>) },
            }
        }
    };

    let is_result = call_function.returns_result_type();
    let is_vec = call_function.returns_vec_type();

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
    let propagate_result = if asynchronous {
        quote! {}
    } else {
        quote! {
            for (channel, count) in return_channels {
                #handle_result
                let data_positions = data_positions.drain(..count).collect::<Vec<_>>();
                if let Some(channel) = channel {
                    let _ = channel.send((result, data_positions)).await;
                }
            }
        }
    };

    let inner_batched = &identifiers.inner_batched;
    let batched_span_name = inner_batched.to_string();
    let executor_batch_data_struct_type = &identifiers.executor_batch_data_struct_type;
    let executor_producer_channel = &identifiers.executor_producer_channel;
    let executor_background_fn = &identifiers.executor_background_fn;

    quote! {
        static #executor_producer_channel:
            ::tokio::sync::OnceCell<::tokio::sync::mpsc::Sender<#channel_type>> = ::tokio::sync::OnceCell::const_new();

        async fn #executor_background_fn() -> ::tokio::sync::mpsc::Sender<#channel_type> {
            let capacity = #capacity;
            let window = #window;
            let window = ::std::time::Duration::from_millis(window as u64);
            #partition_resolver

            let (sender, mut receiver) = ::tokio::sync::mpsc::channel(1);
            ::tokio::task::spawn(async move {
                let semaphore = ::std::sync::Arc::new(::tokio::sync::Semaphore::new(#concurrent_limit));

                #[derive(Debug, Default)]
                struct #executor_batch_data_struct_type {
                    data: Vec<::std::collections::BTreeMap<usize, #arg_type>>,
                    return_channels: Vec<(Option<::tokio::sync::mpsc::Sender<#returned_type_plural>>, usize)>,
                    waiting_spans: Vec<::batched::tracing::Span>,
                }

                let mut batch_data_per_partition: ::std::collections::HashMap::<_, #executor_batch_data_struct_type> = Default::default();
                let mut window_per_partition: ::std::collections::HashMap::<_, (::std::time::Instant, usize)> = Default::default();

                loop {
                    loop {
                        let smallest_remaining_window = window_per_partition.values().map(|(window_start, _)| {
                            let window_end = *window_start + window;
                            window_end.duration_since(::std::time::Instant::now())
                        }).min();
                        let smallest_remaining_window = smallest_remaining_window.unwrap_or(::std::time::Duration::from_hours(1));

                        tokio::select! {
                            event = receiver.recv() => {
                                if event.is_none() {
                                    break;
                                }

                                let event: #channel_type = event.unwrap();
                                let (data_values, origin_span, return_channel) = event;

                                let data_values_with_positions = data_values
                                    .into_iter()
                                    .enumerate()
                                    .collect::<::std::collections::BTreeMap<_, _>>();

                                if let Some(partition_resolver) = &partition_resolver {
                                    let mut per_partition_data: ::std::collections::HashMap<u64, ::std::collections::BTreeMap<_, _>>  = Default::default();
                                    for (position, data_value) in data_values_with_positions {
                                        let partition = partition_resolver(&data_value).await;
                                        per_partition_data
                                            .entry(partition)
                                            .or_default()
                                            .insert(position, data_value);
                                    }

                                    for (partition, data_values_with_positions) in per_partition_data {
                                        let batch_data = batch_data_per_partition.entry(partition).or_default();
                                        let size = data_values_with_positions.len();
                                        batch_data.data.push(data_values_with_positions);
                                        batch_data.return_channels.push((return_channel.clone(), size));
                                        batch_data.waiting_spans.push(origin_span.clone());

                                        if let Some((_, partition_count)) = window_per_partition.get_mut(&partition) {
                                            *partition_count += size;
                                        } else {
                                            window_per_partition.insert(partition, (::std::time::Instant::now(), size));
                                        }
                                    }
                                } else {
                                    let batch_data = batch_data_per_partition.entry(0).or_default();
                                    let size = data_values_with_positions.len();
                                    batch_data.data.push(data_values_with_positions);
                                    batch_data.return_channels.push((return_channel, size));
                                    batch_data.waiting_spans.push(origin_span);

                                    if let Some((_, partition_count)) = window_per_partition.get_mut(&0) {
                                        *partition_count += size;
                                    } else {
                                        window_per_partition.insert(0, (::std::time::Instant::now(), size));
                                    }
                                }

                                let over_capacity_partition = window_per_partition
                                    .values().find(|(_, count)| *count >= capacity);
                                if over_capacity_partition.is_some() {
                                    break;
                                }
                            }

                            _ = ::tokio::time::sleep(smallest_remaining_window) => {
                                break;
                            }
                        }
                    }

                    let ready_partitions = window_per_partition.extract_if(|_, (window_start, count)| {
                        *count >= capacity || (::std::time::Instant::now() - *window_start) >= window
                    }).map(|(partition, _)| partition).collect::<Vec<_>>();
                    if ready_partitions.is_empty() {
                        continue;
                    }

                    let queued_batch_data_per_partition = ready_partitions.into_iter().map(|partition| {
                        (partition, batch_data_per_partition.remove(&partition).unwrap())
                    });

                    for (partition, batch_data) in queued_batch_data_per_partition {
                        let permit = semaphore.clone().acquire_owned().await.unwrap();
                        let data = batch_data.data;
                        let return_channels = batch_data.return_channels;
                        let waiting_spans = batch_data.waiting_spans;
                        tokio::task::spawn(async move {
                            let _permit = permit;
                            let batched_span = ::batched::tracing::info_span!(#batched_span_name, count = data.len(), partition = partition);
                            for mut span in waiting_spans {
                                ::batched::tracing::TracingSpan::link_span(&mut span, &batched_span);
                            }

                            let mut data_positions = vec![];
                            let mut data_values = vec![];
                            for mapped_data_with_position in data {
                                for (position, data) in mapped_data_with_position {
                                    data_positions.push(position);
                                    data_values.push(data);
                                }
                            }

                            let future = #inner_batched(data_values);
                            let future = ::batched::tracing::Instrument::instrument(future, batched_span);
                            let mut result = future.await;
                            #propagate_result
                        });
                    }
                }
            });

            sender
        }
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
    let arg_type = &call_function.batched_arg_type;
    let inner_body = &call_function.inner;
    let returned = &call_function.returned.tokens;

    let is_result = call_function.returns_result_type();
    let is_vec = call_function.returns_vec_type();
    let asynchronous = options.asynchronous;
    let passthrough = options.passthrough;

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
    let resolve_batch_result = if is_result {
        quote! {
            let result = result?;
        }
    } else {
        quote! { let result = result; }
    };
    let return_result = if is_result {
        quote! { Ok(result) }
    } else {
        quote! { result }
    };
    let handle_batch_result = if is_vec {
        quote! {
            let mut mapped_result_with_position: ::std::collections::BTreeMap<usize, _> = Default::default();
            loop {
                let (result, ordering) = response_channel_recv.recv().await
                    .expect("[batched] failed to recieve batch result from executor");
                #resolve_batch_result
                let mut result = result;
                for idx in ordering {
                    let result = result.remove(0);
                    mapped_result_with_position.insert(idx, result);
                }

                if mapped_result_with_position.len() == count {
                    break;
                }
            }

            let result = mapped_result_with_position.into_iter().map(|(_, value)| value).collect();
            #return_result
        }
    } else {
        quote! {
            let mut batch_result = None;
            let mut recieved_results_count = 0;
            loop {
                let (result, positions) = response_channel_recv.recv().await
                    .expect("[batched] failed to recieve batch result from executor");
                #resolve_batch_result
                recieved_results_count += positions.len();

                if let Some(batch_result) = &batch_result {
                    assert_eq!(*batch_result, result, "[batched] partitions cannot return different result values");
                } else {
                    batch_result = Some(result);
                }

                if recieved_results_count == count {
                    break;
                }
            }

            let result = batch_result.unwrap();
            #return_result
        }
    };

    let cast_result_error = match &call_function.returned.result_type {
        FunctionResultType::Raw(_) => None,
        FunctionResultType::VectorRaw(_) => None,
        FunctionResultType::Result(_, _, inner_shared_error) => {
            inner_shared_error.as_ref().map(|inner_shared_error| {
                quote! {
                    let result = result.map_err(|e: #inner_shared_error| e.into());
                }
            })
        }
    };

    let return_single_result = if is_result {
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

    let executor_producer_channel = &identifiers.executor_producer_channel;
    let executor_background_fn = &identifiers.executor_background_fn;
    let inner_batched = &identifiers.inner_batched;
    let inner_passthrough = &identifiers.inner_passthrough;
    let public_interface = &identifiers.public_interface;
    let public_interface_arg = &identifiers.public_interface_arg;
    let public_interface_plural = &identifiers.public_interface_plural;
    let public_interface_plural_arg = &identifiers.public_interface_plural_arg;

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
    let passthrough = if passthrough {
        quote! {
            #(#macros)*
            #visibility async fn #inner_passthrough(#arg) -> #returned {
                let result = async { #inner_body };
                let result = result.await;
                #cast_result_error
                result
            }
        }
    } else {
        quote! {}
    };

    if asynchronous {
        quote! {
            #inner_batched
            #passthrough

            #tracing_span
            #visibility async fn #public_interface(#public_interface_arg: #arg_type) {
                #public_interface_plural(vec![#public_interface_arg]).await;
            }

            #tracing_span
            #visibility async fn #public_interface_plural(#public_interface_plural_arg: Vec<#arg_type>) {
                let channel = &#executor_producer_channel;
                let channel = channel.get_or_init(async || { #executor_background_fn().await }).await;

                let span = ::batched::tracing::Span::current();
                channel.send((#public_interface_plural_arg, span, None)).await
                    .expect("[batched] failed to batch to executor");
            }
        }
    } else {
        quote! {
            #inner_batched
            #passthrough

            #tracing_span
            #visibility async fn #public_interface(#public_interface_arg: #arg_type) -> #return_type {
                let mut result = #public_interface_plural(vec![#public_interface_arg]).await;
                #return_single_result
            }

            #tracing_span
            #visibility async fn #public_interface_plural(#public_interface_plural_arg: Vec<#arg_type>) -> #return_type_multiple {
                let channel = &#executor_producer_channel;
                let channel = channel.get_or_init(async || { #executor_background_fn().await }).await;
                let count = #public_interface_plural_arg.len();

                let (response_channel_sender, mut response_channel_recv) = ::tokio::sync::mpsc::channel(1);
                let span = ::batched::tracing::Span::current();
                channel.send((#public_interface_plural_arg, span, Some(response_channel_sender))).await
                    .expect("[batched] failed to batch to executor");

                #handle_batch_result
            }
        }
    }
}
