use gendock::{paths::Paths, store::Store};
use serde_json::json;

#[test]
fn independent_connections_enforce_pending_limit_and_exclusive_fifo_claim() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(dir.path(), |_| None);
    Store::open(&paths.database()).unwrap();
    let admitted = std::thread::scope(|scope| {
        (0..20)
            .map(|index| {
                let db = paths.database();
                scope.spawn(move || {
                    Store::open(&db).unwrap().submit_service(
                        "image.generate",
                        &json!({"prompt":index}),
                        "alice",
                        5,
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|handle| handle.join().unwrap().ok())
            .collect::<Vec<_>>()
    });
    assert_eq!(admitted.len(), 5);
    let claimed = std::thread::scope(|scope| {
        (0..8)
            .map(|_| {
                let db = paths.database();
                scope.spawn(move || {
                    Store::open(&db)
                        .unwrap()
                        .claim_service("instance-one")
                        .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(claimed.len(), 1);
    let id = claimed[0].id;
    assert_eq!(id, *admitted.iter().min().unwrap());
    let store = Store::open(&paths.database()).unwrap();
    assert!(store.service_tick(id, "foreign-instance").is_err());
    assert!(!store.service_tick(id, "instance-one").unwrap());
    assert!(store.cancel_service(id, "bob", false).unwrap().is_none());
    store.cancel_service(id, "alice", false).unwrap();
    assert!(store.service_tick(id, "instance-one").unwrap());
    // Cancellation racing with a busy GPU must not strand a running job or requeue it.
    store.requeue_service(id).unwrap();
    assert_eq!(store.job_info(id).unwrap().unwrap()["status"], "cancelled");
    let mut expected = admitted;
    expected.sort_unstable();
    assert_eq!(
        store.claim_service("instance-two").unwrap().unwrap().id,
        expected[1]
    );
    assert_eq!(store.visible_jobs("alice", false).unwrap().len(), 5);
    assert!(store.visible_jobs("bob", false).unwrap().is_empty());
}
