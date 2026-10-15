//! What a public event costs the core: nothing built for an event nobody
//! subscribed to, one queue push for one that has a subscriber.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::json;
use vorn_extensions::events::{Bus, SESSION_EXITED, SESSION_MARK};

fn emit(c: &mut Criterion) {
    let bus = Bus::new();
    let mut rx = bus.subscribe("bench", &[SESSION_EXITED]);
    c.bench_function("emit with no subscriber", |b| {
        b.iter(|| bus.emit(black_box(SESSION_MARK), || json!({ "sessionId": "s", "phase": "prompt" })))
    });
    c.bench_function("emit to one subscriber", |b| {
        b.iter(|| {
            bus.emit(black_box(SESSION_EXITED), || json!({ "sessionId": "s" }));
            while rx.try_recv().is_ok() {}
        })
    });
}

criterion_group!(benches, emit);
criterion_main!(benches);
