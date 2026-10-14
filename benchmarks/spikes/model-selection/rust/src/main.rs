//! Usage: `model-selection-bench <model-dir> <names.txt> <descriptions.txt> [--warm]`.
//!
//! Without `--warm`, loads the model once and prints its first load time,
//! dimension and median throughput over three runs after a warmup, so that
//! the process's peak memory counts one model. With `--warm`, loads, drops
//! and reloads the model and prints only the second load time. Threads
//! follow `RAYON_NUM_THREADS`.

use std::time::Instant;

use model2vec_rs::model::StaticModel;

fn lines(path: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("readable text file")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn load(path: &str) -> (StaticModel, f64) {
    let start = Instant::now();
    let model = StaticModel::from_pretrained(path, None, None, None).expect("model loads");
    (model, start.elapsed().as_secs_f64())
}

fn throughput(model: &StaticModel, texts: &[String]) -> f64 {
    model.encode(&texts[..64.min(texts.len())]);
    let mut runs: Vec<f64> = (0..3)
        .map(|_| {
            let start = Instant::now();
            let vectors = model.encode(texts);
            assert_eq!(vectors.len(), texts.len());
            texts.len() as f64 / start.elapsed().as_secs_f64()
        })
        .collect();
    runs.sort_by(f64::total_cmp);
    runs[1]
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    match arguments.as_slice() {
        [_, model_dir, _, _, warm] if warm == "--warm" => {
            drop(load(model_dir));
            let (_, warm_load_s) = load(model_dir);
            println!("{{\"warm_load_s\": {warm_load_s}}}");
        }
        [_, model_dir, names, descriptions] => {
            let (model, first_load_s) = load(model_dir);
            let dimension = model.encode(&["probe".to_owned()])[0].len();
            let names_per_s = throughput(&model, &lines(names));
            let descriptions_per_s = throughput(&model, &lines(descriptions));
            println!(
                "{{\"first_load_s\": {first_load_s}, \"dimension\": {dimension}, \
                 \"names_per_s\": {names_per_s}, \"descriptions_per_s\": {descriptions_per_s}}}"
            );
        }
        _ => panic!("usage: model-selection-bench <model-dir> <names.txt> <descriptions.txt> [--warm]"),
    }
}
