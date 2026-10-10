#[path = "../examples/support/manual_timeout.rs"]
mod manual_timeout;
use manual_timeout::{Expired, Timeout};
use std::time::Duration;
#[tokio::test]
async fn immediately_ready_inner_wins() {
    assert_eq!(
        Timeout::new(Duration::from_secs(1), async { 42 }).await,
        Ok(42)
    );
}
#[tokio::test]
async fn timer_wakes_pending_future() {
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(1),
            Timeout::new(Duration::from_millis(10), std::future::pending::<()>())
        )
        .await
        .unwrap(),
        Err(Expired)
    );
}
#[tokio::test]
async fn inner_can_wake_and_finish_first() {
    assert_eq!(
        Timeout::new(Duration::from_secs(1), async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            7
        })
        .await,
        Ok(7)
    );
}
