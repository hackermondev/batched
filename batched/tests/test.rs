use std::time::Instant;

use batched::batched;

#[tokio::test]
async fn simple() {
    #[batched(window = 100, limit = 1000)]
    fn add(numbers: Vec<u32>) -> u32 {
        numbers.iter().sum()
    }

    for _ in 0..99 {
        tokio::task::spawn(async move { 
            add_multiple(vec![1, 1, 1]).await
        });
    }

    let total = add_multiple(vec![1, 1, 1]).await;
    let expected_total = 100 * 3;
    assert_eq!(total, expected_total);
}

#[tokio::test]
async fn propagates_errors() {
    #[batched(window = 100, limit = 1000, boxed)]
    fn error(_a: Vec<()>) -> Result<(), std::io::Error> {
        return Err(std::io::Error::other("1234")).into();
    }

    let result = error(()).await;
    assert_eq!(result.is_err(), true);
}

#[tokio::test]
async fn batched_window_works() {
    #[batched(window = 1000, limit = 1000)]
    fn add(numbers: Vec<u32>) -> u32 {
        numbers.iter().sum()
    }

    let before = Instant::now();
    add_multiple(vec![1, 1, 1]).await;
    let after = before.elapsed();
    assert!(after.as_secs() == 1);
}