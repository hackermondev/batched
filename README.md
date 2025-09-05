# batched
Rust macro utility for batching expensive async operations.

## What is this?
`batched` is designed for high-throughput async environments where many small, frequent calls would otherwise overwhelm your system or database. Instead of processing each call individually, it groups them into batches based on configurable rules (time window, size limit, concurrency), then executes a single batched operation.  

This saves resources, reduces contention, and improves efficiency — all while letting callers use the function as if it were unbatched.  

You annotate an async function with `#[batched]`, and the macro generates the batching logic automatically.

---

## When it’s useful (and when it’s not)

### ✅ Useful
- **Database inserts/updates:** Instead of writing one row at a time, batch them into multi-row `INSERT` or `UPDATE` statements.
- **External API calls with rate limits:** Reduce request overhead by batching multiple logical calls into one HTTP request.
- **Expensive computations:** Grouping repeated small computations into a single parallel-friendly call.
- **Services with bursts of traffic:** Smooth out request spikes by accumulating calls into fewer batch operations.

### ❌ Not useful
- **Lightweight or fast operations**: If the work per call is already cheap (e.g. adding two numbers), batching only adds complexity and overhead.
- **Strong ordering or per-call timing guarantees required**: Calls may be delayed slightly while waiting for the batch window.

## Installation
```sh
cargo add batched 
```

Or add this to your `Cargo.toml`:
```toml
[dependencies]
batched = "0.2.8"
```

### Nightly Rust
Due to the use of advanced features, `batched` requires a nightly Rust compiler. 


## #[batched]
- **limit**: Maximum amount of items that can be grouped and processed in a single batch. (required)
- **concurrent**: Maximum amount of concurrent batched tasks running (default: `Infinity`)
- **asynchronous**: If true, the caller does not wait for the batch to complete, and the return value is `()`. (default: `false`).
- **window**: Maximum amount of time (in milliseconds) the background thread waits after the first call before processing a batch. (required)
- **window[x]**: Maximum amount of time (in milliseconds) the background thread waits after the first call before processing a batch, when the buffer size is <= x. (This allows for more granular control of the batching window based on the current load. For example, you might want to use a shorter window when there are fewer items in the buffer to reduce latency, and a longer window when there are more items to maximize batching efficiency.)



The target function must have a single input argument, a vector of items (`Vec<T>`). 

The return value of the batched function is propagated (cloned) to all async calls of the batch, unless the batched function returns a `Vec<T>`, in which case the return value for each call is pulled from the iterator in the same order of the input.

If the return value is not an iterator, The target function return type must implement `Clone` to propagate the result. Use `batched::error::SharedError` to wrap your error types (if they don't implement Clone).


## Prerequisites 
- Built for async environments (tokio), will not work without a tokio async runtime
- The target function must be an async function
- Not supported inside structs:
```rust
struct A;

impl A {
    #[batched(window = 1000, limit = 100)]
    fn operation() {
        ...
    }
}
```

## Tracing
### [`tracing_span`]
This feature automatically adds tracing spans to call functions for batched requests (`x`, `x_multiple`).

### [`tracing_opentelemetry`]
This feature adds support for linking spans from callers to the inner batched call when using OpenTelemetry. Depending on whether your OpenTelemetry client supports it, you should be able to see the linked span to the batched call. 

## Examples

### Simple add batch
```rust
#[batched(window = 100, limit = 1000)]
async fn add(numbers: Vec<u32>) -> u32 {
    numbers.iter().sum()
}

async fn main() {
    for _ in 0..99 {
        tokio::task::spawn(async move {
            add(1).await
        });
    }

    let result = add(1).await;
    assert_eq!(result, 100);
}
```

### Batch insert rows

```rust
use batched::{batched, error::SharedError};

// Macros creates functions [`insert_message`] and [`insert_message_multiple`]
#[batched(window = 100, window1 = 10, window5 = 20, limit = 100_000)]
async fn insert_message(messages: Vec<String>) -> Result<(), SharedError<anyhow::Error>> {
    let pool = PgPool::connect("postgres://user:password@localhost/dbname").await?;
    let mut query = String::from("INSERT INTO messages (content) VALUES ");
    ...
}

#[post("/message")]
async fn service(message: String) -> Result<(), anyhow::Error> {
    insert_message(message).await?;
    Ok(())
}

#[post("/bulk_messages")]
async fn service(messages: Vec<String>) -> Result<(), anyhow::Error> {
    insert_message_multiple(messages).await?;
    Ok(())
}
```

### Batch insert rows and return them

```rust
use batched::{batched, error::SharedError};

struct Row {
    pub id: usize,
    pub content: String,
}

// Macros creates functions [`insert_message`] and [`insert_message_multiple`]
#[batched(window = 100, window1 = 10, window5 = 20, limit = 100_000)]
async fn insert_message_batched(messages: Vec<String>) -> Result<Vec<Row>, SharedError<anyhow::Error>> {
    let pool = PgPool::connect("postgres://user:password@localhost/dbname").await?;
    let mut query = String::from("INSERT INTO messages (content) VALUES ");
    ...
}

#[post("/message")]
async fn service(message: String) -> Result<(), anyhow::Error> {
    let message: Row = insert_message(message).await?;
    Ok(())
}

#[post("/bulk_messages")]
async fn service(messages: Vec<String>) -> Result<(), anyhow::Error> {
    let messages: Vec<Row> = insert_message_multiple(messages).await?;
    Ok(())
}
```
