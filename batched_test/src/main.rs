use std::time::Duration;

use batched::batched;

#[batched(window = 1000, limit = 1000)]
fn call(numbers: Vec<u32>) -> () {
    println!("woah, this is running inside a batch! {numbers:?}");
}

#[tokio::main]
async fn main() {
    println!("hello world");
    for _ in 0..100 {
        tokio::task::spawn(async move {
            let result = call_multiple(vec![1, 2, 3]).await;
            println!("recv result: {result:?}");
        });
    }

    tokio::time::sleep(Duration::from_secs(60)).await;
}