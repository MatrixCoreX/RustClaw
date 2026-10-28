use super::*;
use std::time::{Duration, Instant};

#[path = "persistence_tests.rs"]
mod persistence_contracts;

fn single_connection_fixture() -> (AppState, ClaimedTask, Value, ChannelDeliveryEnvelope) {
    let (mut state, task, payload, envelope) = fixture();
    state.core.db = r2d2::Pool::builder()
        .max_size(1)
        .connection_timeout(Duration::from_secs(3))
        .build(r2d2_sqlite::SqliteConnectionManager::memory())
        .unwrap();
    let state = state.with_seeded_db_schema();
    {
        let db = state.core.db.get().unwrap();
        crate::repo::channel_delivery_receipt::ensure_channel_delivery_receipt_schema(&db).unwrap();
    }
    (state, task, payload, envelope)
}

#[tokio::test(flavor = "current_thread")]
async fn cold_dispatch_claim_pool_wait_keeps_runtime_responsive() {
    let (state, task, payload, envelope) = single_connection_fixture();
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let (result, timer_delay) = tokio::join!(
        crate::delivery_service::deliver_task_envelope_once(
            &state, &task, &payload, &envelope, true
        ),
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    release.join().unwrap();
    assert_eq!(result.unwrap().status, ChannelDeliveryServiceStatus::Failed);
    eprintln!(
        "stage=cold_claim pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
}

#[derive(Clone)]
struct ReceiptWaitMock {
    state: AppState,
    started: Arc<tokio::sync::Notify>,
    timer_start: Arc<std::sync::Mutex<Option<Instant>>>,
    sends: Arc<AtomicUsize>,
}

async fn accept_while_database_is_busy(State(mock): State<ReceiptWaitMock>) -> Json<Value> {
    let held = mock.state.core.db.get().unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    *mock.timer_start.lock().unwrap() = Some(Instant::now());
    mock.started.notify_one();
    mock.sends.fetch_add(1, Ordering::SeqCst);
    Json(json!({"ok":true,"message_ids":["receipt-busy-1"]}))
}

#[tokio::test(flavor = "current_thread")]
async fn post_send_receipt_pool_wait_keeps_runtime_responsive_without_replay() {
    let (mut state, task, payload, envelope) = single_connection_fixture();
    let started = Arc::new(tokio::sync::Notify::new());
    let timer_start = Arc::new(std::sync::Mutex::new(None));
    let sends = Arc::new(AtomicUsize::new(0));
    let mock = ReceiptWaitMock {
        state: state.clone(),
        started: started.clone(),
        timer_start: timer_start.clone(),
        sends: sends.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_web_bridge_base_url =
        format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v1/send-text", post(accept_while_database_is_busy))
        .with_state(mock);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (result, timer_delay) = tokio::join!(
        crate::delivery_service::deliver_task_envelope_once(
            &state, &task, &payload, &envelope, true
        ),
        async {
            started.notified().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
            timer_start.lock().unwrap().unwrap().elapsed()
        }
    );
    server.abort();
    let _ = server.await;
    assert!(result.unwrap().accepted());
    let replay = crate::delivery_service::deliver_task_envelope_once(
        &state, &task, &payload, &envelope, true,
    )
    .await
    .unwrap();
    assert!(replay.accepted());
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    eprintln!(
        "stage=post_send_receipt pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
}
