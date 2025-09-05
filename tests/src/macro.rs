use std::{sync::{atomic::AtomicBool, LazyLock}, time::{Duration, Instant}};

use batched::{batched, error::SharedError};

#[tokio::test]
async fn simple() {
    #[batched(window1 = 10, window = 100, limit = 1000)]
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
    #[batched(window = 100, limit = 1000)]
    fn error(_a: Vec<()>) -> Result<(), SharedError<std::io::Error>> {
        Err(std::io::Error::other("1234").into())
    }

    let result = error(()).await;
    assert!(result.is_err());
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
async fn asynchronous() {
    static BACKGROUND_FN_RAN: LazyLock<AtomicBool> = LazyLock::new(|| 
        AtomicBool::new(false)
    );

    #[batched(window = 500, limit = 1000, asynchronous)]
    fn add(numbers: Vec<u32>) {
        let _sum = numbers.iter().sum::<u32>();
        BACKGROUND_FN_RAN.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let instant = Instant::now();
    add(1).await;
    assert!(instant.elapsed().as_millis() < 5);

    tokio::time::sleep(Duration::from_secs(1)).await;
    let background_fn_ran = BACKGROUND_FN_RAN.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(background_fn_ran, true);
}

#[tokio::test]
async fn window() {
    #[batched(window = 1000, window2 = 10, limit = 1000)]
    fn add(numbers: Vec<u32>) -> u32 {
        numbers.iter().sum()
    }

    let start = Instant::now();
    add_multiple(vec![1, 1]).await;
    let elapsed = start.elapsed();
    println!("{elapsed:?}");
    assert!(elapsed.as_millis() <= 15);

    let start = Instant::now();
    add_multiple(vec![1, 1, 1]).await;
    let elapsed = start.elapsed();
    println!("{elapsed:?}");
    assert!(elapsed.as_secs() == 1);
}

#[tokio::test]
async fn returned_iterator() {
    #[batched(window = 100, limit = 1000)]
    fn add_each(numbers: Vec<u32>) -> Vec<u32> {
        numbers.into_iter().map(|n| n + 1).collect()
    }

    let result = add_each_multiple(vec![1, 1, 1]).await;
    assert!(result == vec![2, 2, 2]);

    let result = add_each(2).await;
    assert!(result == 3);
}

#[tokio::test]
async fn returned_iterator_with_error() {
    #[batched(window = 100, limit = 1000)]
    fn add_each(numbers: Vec<u32>) -> Result<Vec<u32>, SharedError<()>> {
        Ok(numbers.into_iter().map(|n| n + 1).collect())
    }

    let result = add_each_multiple(vec![1, 1, 1]).await.unwrap();
    assert!(result == vec![2, 2, 2]);

    let result = add_each(2).await.unwrap();
    assert!(result == 3);
}