use std::time::{Duration, Instant};

use batched::batched;

#[tokio::test]
async fn simple() {
    #[batched(window = 100, limit = 1000)]
    fn add(numbers: Vec<u32>) -> u32 {
        numbers.iter().sum()
    }

    for _ in 0..99 {
        tokio::task::spawn(async move { add_multiple(vec![1, 1, 1]).await });
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
async fn empty_batch() {
    #[batched(window = 100, limit = 1000)]
    fn add(numbers: Vec<u32>) -> u32 {
        numbers.iter().sum()
    }

    let timeout = tokio::time::timeout(Duration::from_secs(1), add_multiple(vec![])).await;
    timeout.expect("batch timed out");
}

#[tokio::test]
async fn batched_window() {
    #[batched(window = 1000, limit = 1000)]
    fn add(numbers: Vec<u32>) -> u32 {
        numbers.iter().sum()
    }

    let before = Instant::now();
    add_multiple(vec![1, 1, 1]).await;
    let after = before.elapsed();
    assert!(after.as_secs() == 1);
}

#[tokio::test]
async fn batched_with_returned_iterator() {
    #[batched(window = 100, limit = 1000, iterator_value = u32)]
    fn expensive_task(numbers: Vec<u32>) -> Vec<u32> {
        numbers.into_iter().map(|n| n + 1).collect()
    }

    let input = vec![1, 1, 1];
    let result = expensive_task_multiple(input).await;
    assert!(result == vec![2, 2, 2]);

    let result = expensive_task(2).await;
    assert!(result == 3);
}