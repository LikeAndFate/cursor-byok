//! Exercises the safe display projection independently of model and Cursor history.
use cursor_server::cursor::{
    protocol::{connect, proto::agent::v1 as pb},
    transport::OutputHub,
};

#[test]
fn display_tap_never_changes_cursor_frames_or_blocks_without_a_phone() {
    let hub = OutputHub::default();
    let message = pb::AgentServerMessage {
        ttft_breakdown: None,
        message: Some(pb::agent_server_message::Message::InteractionUpdate(
            pb::InteractionUpdate {
                message: Some(pb::interaction_update::Message::TextDelta(
                    pb::TextDeltaUpdate {
                        text: "hello <script> text".into(),
                        is_server_notice: false,
                    },
                )),
            },
        )),
    };
    let frame = connect::encode_message(&message).unwrap();
    assert!(hub.emit(frame.clone()));
    let mut subscriber = hub.subscribe();
    assert_eq!(subscriber.try_recv().unwrap(), frame);
    assert!(hub.close());
    assert!(!hub.emit(connect::encode_message(&message).unwrap()));
}
