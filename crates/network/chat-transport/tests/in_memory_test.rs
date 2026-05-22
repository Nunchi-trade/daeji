//! Integration tests for [`InMemoryChat`].
//!
//! These cover the contract every [`ChatTransport`] impl must honor:
//! publish/subscribe roundtrip, channel isolation, multi-subscriber
//! fan-out, and the connected-set abstraction used by the local
//! symphony demo.

use bytes::Bytes;
use chat_transport::{ChatTransport, InMemoryChat, PeerInfo};
use futures::StreamExt;

fn peer(name: &str) -> PeerInfo {
    PeerInfo { pubkey: name.as_bytes().to_vec(), addr: Some(format!("inmem://{name}")) }
}

#[tokio::test(flavor = "current_thread")]
async fn publish_subscribe_roundtrip() {
    let t = InMemoryChat::new(peer("alice"));
    let mut sub = t.subscribe(42);
    t.publish(42, Bytes::from_static(b"hello")).await.expect("publish");
    let frame = sub.next().await.expect("frame");
    assert_eq!(frame.channel_id, 42);
    assert_eq!(frame.sender.pubkey, b"alice");
    assert_eq!(&frame.payload[..], b"hello");
}

#[tokio::test(flavor = "current_thread")]
async fn channels_are_isolated() {
    let t = InMemoryChat::new(peer("alice"));
    let mut sub_lobby = t.subscribe(1);
    let mut sub_slot = t.subscribe(2);
    t.publish(1, Bytes::from_static(b"lobby")).await.expect("publish");
    t.publish(2, Bytes::from_static(b"slot")).await.expect("publish");

    let f1 = sub_lobby.next().await.expect("lobby frame");
    let f2 = sub_slot.next().await.expect("slot frame");
    assert_eq!(&f1.payload[..], b"lobby");
    assert_eq!(&f2.payload[..], b"slot");
}

#[tokio::test(flavor = "current_thread")]
async fn connected_set_sees_each_others_traffic() {
    // Four agents on a shared backing — the local-symphony-demo shape.
    let agents = InMemoryChat::connected_set(&["alice", "bob", "carol", "dave"]);
    assert_eq!(agents.len(), 4);

    // Bob, Carol, Dave subscribe to channel 7.
    let mut sub_b = agents[1].subscribe(7);
    let mut sub_c = agents[2].subscribe(7);
    let mut sub_d = agents[3].subscribe(7);

    // Alice publishes.
    agents[0].publish(7, Bytes::from_static(b"hi from alice")).await.expect("publish");

    for sub in [&mut sub_b, &mut sub_c, &mut sub_d] {
        let f = sub.next().await.expect("frame");
        assert_eq!(f.sender.pubkey, b"alice");
        assert_eq!(&f.payload[..], b"hi from alice");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn peers_includes_all_connected_identities() {
    let agents = InMemoryChat::connected_set(&["alice", "bob", "carol"]);
    let peers = agents[0].peers();
    let names: Vec<_> = peers.iter().map(|p| p.pubkey.clone()).collect();
    assert!(names.contains(&b"alice".to_vec()));
    assert!(names.contains(&b"bob".to_vec()));
    assert!(names.contains(&b"carol".to_vec()));
}

#[tokio::test(flavor = "current_thread")]
async fn register_channels_is_idempotent() {
    let t = InMemoryChat::new(peer("alice"));
    t.register_channels(&[1, 2, 3]).await.expect("first");
    t.register_channels(&[1, 2, 3]).await.expect("second");

    // Subsequent publishes work as expected.
    let mut sub = t.subscribe(2);
    t.publish(2, Bytes::from_static(b"ok")).await.expect("publish");
    let f = sub.next().await.expect("frame");
    assert_eq!(&f.payload[..], b"ok");
}

#[tokio::test(flavor = "current_thread")]
async fn local_identity_matches_publisher() {
    let (a, _b) = InMemoryChat::connected_pair("alice", "bob");
    assert_eq!(a.local_identity().pubkey, b"alice");
    let mut sub_a = a.subscribe(99);
    a.publish(99, Bytes::from_static(b"x")).await.expect("publish");
    let f = sub_a.next().await.expect("frame");
    assert_eq!(f.sender.pubkey, b"alice");
}
