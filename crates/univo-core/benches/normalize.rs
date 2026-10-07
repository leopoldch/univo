use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use univo_core::{normalize_hostname, normalize_text};

const ASCII_NAME: &str = "Amazon Web Services";
const ASCII_DESCRIPTION: &str = "Online retail, cloud computing, digital streaming and artificial \
     intelligence services for consumers and businesses worldwide";
const FRENCH_DESCRIPTION: &str = "Société française spécialisée dans la grande distribution, \
     hypermarchés, supermarchés et commerce électronique à l'échelle européenne";
const DECOMPOSED: &str = "Cafe\u{301} de Flore, Sa\u{300}o Paulo, Zu\u{308}rich";

fn text(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize_text");
    for (name, input) in [
        ("ascii_name", ASCII_NAME),
        ("ascii_description", ASCII_DESCRIPTION),
        ("french_description", FRENCH_DESCRIPTION),
        ("decomposed", DECOMPOSED),
    ] {
        group.throughput(Throughput::Bytes(input.len() as u64));
        let mut out = String::with_capacity(input.len() * 3);
        group.bench_function(name, |b| {
            b.iter(|| {
                out.clear();
                black_box(normalize_text(black_box(input), true, &mut out));
            });
        });
    }
    group.finish();
}

fn hostname(c: &mut Criterion) {
    let mut out = String::with_capacity(64);
    c.bench_function("normalize_hostname", |b| {
        b.iter(|| {
            out.clear();
            black_box(normalize_hostname(black_box("WWW.Example.COM."), &mut out)).unwrap();
        });
    });
}

criterion_group!(benches, text, hostname);
criterion_main!(benches);
