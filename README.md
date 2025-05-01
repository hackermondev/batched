# batched
Rust macro utility for batching expensive async operations.

## Installation
```sh
cargo add batched 
```

Or add this to your `Cargo.toml`:
```toml
[dependencies]
batched = "0.1.0"
```

## #[batched]
- **window**: Minimum amount of time (in milliseconds) the background thread waits before processing a batch.
- **limit**: Maximum amount of items that can be grouped and processed in a single batch.

The target function must have a single argument, a vector of items (`Vec<T>`). The return value (must implement `Clone`) is propagated to all async calls made for the batch items. 

If the target function returns a `Result<T, E>`, the generics must implement `Clone` or the return must wrapped in `Arc<T>`.

## Prerequisites 
- Built for async environments (tokio), will not work without a tokio async runtime
- Not supported inside structs
```rust
struct A;

impl A {
    // NOT SUPPORTED
    #[batched(window = 1000, limit = 100)]
    fn operation() {
        ...
    }
}
```



## Examples

### Batch insert Postgres rows

```rust
use batched::batched;

// Creates functions [`insert_message`] and [`insert_message_multiple`]
#[batched(window = 100, limit = 100_000)]
async fn insert_message_batched(messages: Vec<String>) -> Arc<Result<(), anyhow::Error>> {
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

