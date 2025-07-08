# batched
Rust macro utility for batching expensive async operations.

## Installation
```sh
cargo add batched 
```

Or add this to your `Cargo.toml`:
```toml
[dependencies]
batched = "0.2.1"
```

## #[batched]
- **window**: Minimum amount of time (in milliseconds) the background thread waits before processing a batch.
- **limit**: Maximum amount of items that can be grouped and processed in a single batch.
- **concurrent**: Maximum amount of concurrent batched tasks running (default: `Infinity`)

The target function must have a single argument, a vector of items (`Vec<T>`). 

The return value of the batched function is propagated (cloned) to all async calls of the batch, unless the batched function returns a `Vec<T>`, in which case the return value for each call is pulled from the iterator in the same order of the input.

If the return value is not an iterator, The target function return type must implement `Clone` to propagate the result. Use `batched::error::SharedError` to wrap your error types (if they don't implement Clone).


## Prerequisites 
- Built for async environments (tokio), will not work without a tokio async runtime
- Target function must have async
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
#[batched(window = 100, limit = 100_000, boxed)]
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

#[batched(window = 100, limit = 100_000)]
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