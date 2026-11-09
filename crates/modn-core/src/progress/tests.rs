use super::*;
use std::sync::Mutex;

fn recording(token: CancellationToken) -> (OperationControl, Arc<Mutex<Vec<ProgressEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let control = OperationControl::new(token, move |event| {
        sink.lock().expect("event log lock").push(event.clone());
    });
    (control, events)
}

fn extract() -> ProgressEvent {
    ProgressEvent::Extract {
        package: Some("main/game/mod".to_owned()),
    }
}

#[test]
fn events_serialize_as_one_object_with_a_phase() {
    let cases = [
        (ProgressEvent::Resolve, r#"{"phase":"resolve"}"#),
        (
            ProgressEvent::Download {
                package: Some("main/game/mod".to_owned()),
                bytes: 10,
                total: None,
            },
            r#"{"phase":"download","package":"main/game/mod","bytes":10,"total":null}"#,
        ),
        (
            ProgressEvent::Stage { package: None },
            r#"{"phase":"stage"}"#,
        ),
        (
            ProgressEvent::Result {
                outcome: Outcome::Cancelled,
                error: Some(ErrorCode::Cancelled),
            },
            r#"{"phase":"result","outcome":"cancelled","error":"CANCELLED"}"#,
        ),
        (
            ProgressEvent::Result {
                outcome: Outcome::Succeeded,
                error: None,
            },
            r#"{"phase":"result","outcome":"succeeded"}"#,
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(serde_json::to_string(&event).expect("event JSON"), expected);
    }
}

#[test]
fn cancellation_stops_before_the_commit_point_without_reporting() {
    let token = CancellationToken::new();
    let (control, events) = recording(token.clone());
    control.advance(ProgressEvent::Resolve).expect("not cancelled");
    token.cancel();
    assert_eq!(control.advance(extract()), Err(Cancelled));
    assert_eq!(
        control.advance(ProgressEvent::Commit { package: None }),
        Err(Cancelled)
    );
    assert!(!control.is_committed());
    assert_eq!(*events.lock().expect("events"), [ProgressEvent::Resolve]);
}

#[test]
fn cancellation_after_the_commit_point_is_ignored() {
    let token = CancellationToken::new();
    let (control, events) = recording(token.clone());
    control
        .advance(ProgressEvent::Commit { package: None })
        .expect("not cancelled");
    token.cancel();
    assert_eq!(control.checkpoint(), Ok(()));
    control.advance(extract()).expect("committed operations finish");
    assert!(control.is_committed());
    assert_eq!(events.lock().expect("events").len(), 2);
}

#[test]
fn result_events_are_reported_even_when_cancelled() {
    let token = CancellationToken::new();
    let (control, events) = recording(token.clone());
    token.cancel();
    let result = ProgressEvent::Result {
        outcome: Outcome::Cancelled,
        error: Some(ErrorCode::Cancelled),
    };
    control.advance(result.clone()).expect("results are not checked");
    assert_eq!(*events.lock().expect("events"), [result]);
}

#[test]
fn clones_share_the_token_and_the_commit_point() {
    let control = OperationControl::default();
    let clone = control.clone();
    clone
        .advance(ProgressEvent::Commit { package: None })
        .expect("not cancelled");
    control.token().cancel();
    assert!(clone.token().is_cancelled());
    assert_eq!(control.checkpoint(), Ok(()));
}
