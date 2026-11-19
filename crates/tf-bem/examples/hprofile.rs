//! Time the pieces of the compressed solver on a TFM1 mesh:
//!   cargo run --release -p tf-bem --example hprofile -- mesh.bin [nodes.f64] [eta] [leaf] [precond]
use std::time::Instant;
use tf_bem::hmatrix::{BlockJacobi, HMatrix, HParams};
use tf_bem::{Conductor, Problem};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).unwrap();
    let u = |o: usize| u32::from_le_bytes(bin[o..o + 4].try_into().unwrap());
    let (nn, nt) = (u(4) as usize, u(8) as usize);
    let mut nodes: Vec<f64> = (0..3 * nn).map(|i| f32::from_le_bytes(bin[12 + 4 * i..16 + 4 * i].try_into().unwrap()) as f64).collect();
    if let Some(raw) = a.get(2).and_then(|p| std::fs::read(p).ok()) {
        nodes = raw.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
    }
    let o = 12 + 12 * nn;
    let tris: Vec<u32> = (0..3 * nt).map(|i| u(o + 4 * i)).collect();
    let p = Problem::from_mesh(&nodes, &tris, vec![0; nt], vec![Conductor { name: "x".into(), floating: false }]).unwrap();
    let hp = HParams {
        eta: a.get(3).map(|s| s.parse().unwrap()).unwrap_or(1.5),
        leaf_size: a.get(4).map(|s| s.parse().unwrap()).unwrap_or(32),
        aca_eps: 1e-6,
    };
    let pb: usize = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(256);
    let t = Instant::now();
    let h = HMatrix::build(&p.panels, &hp);
    println!("build {:.3} s  {:?}", t.elapsed().as_secs_f64(), h.stats);
    let t = Instant::now();
    let pre = BlockJacobi::build(&p.panels, &h.tree, pb);
    println!("precond build {:.3} s", t.elapsed().as_secs_f64());
    let n = nt;
    for k in [1, 6] {
        let x: Vec<f64> = (0..n * k).map(|i| ((i * 7919) % 1000) as f64 / 1000.0).collect();
        let mut y = vec![0.0; n * k];
        h.matvec(&x, &mut y, k);
        let reps = 20;
        let t = Instant::now();
        for _ in 0..reps {
            h.matvec(&x, &mut y, k);
        }
        let dt = t.elapsed().as_secs_f64() / reps as f64;
        println!("matvec k={k}: {:.2} ms  ({:.1} GB/s of matrix data)", dt * 1e3, h.stats.bytes / dt / 1e9);
        let mut z = x.clone();
        let t = Instant::now();
        for _ in 0..reps {
            pre.apply(&mut z, n, k);
        }
        println!("precond k={k}: {:.2} ms", t.elapsed().as_secs_f64() / reps as f64 * 1e3);
    }
    // a full 6-RHS GMRES solve with time spent in the operator and preconditioner
    use std::sync::Mutex;
    let (t_op, t_pre, n_op) = (Mutex::new(0.0f64), Mutex::new(0.0f64), Mutex::new(0usize));
    let op = |x: &[f64], y: &mut [f64], k: usize| {
        let t = Instant::now();
        h.matvec(x, y, k);
        *t_op.lock().unwrap() += t.elapsed().as_secs_f64();
        *n_op.lock().unwrap() += 1;
    };
    let prec = |x: &mut [f64], k: usize| {
        let t = Instant::now();
        pre.apply(x, n, k);
        *t_pre.lock().unwrap() += t.elapsed().as_secs_f64();
    };
    let k = 6;
    let b: Vec<f64> = (0..n * k).map(|i| ((i / n) == (i % 6)) as u8 as f64 + 0.1).collect();
    let mut x = vec![0.0; n * k];
    let t = Instant::now();
    let s = tf_bem::iterative::gmres(n, k, &b, &mut x, &op, &prec, &tf_bem::GmresParams::default(), &|_, _| {}, &|| false);
    let total = t.elapsed().as_secs_f64();
    let (o, pr) = (*t_op.lock().unwrap(), *t_pre.lock().unwrap());
    println!(
        "gmres: total {:.3} s = operator {:.3} s ({} calls) + preconditioner {:.3} s + other {:.3} s; iterations {:?}",
        total,
        o,
        n_op.lock().unwrap(),
        pr,
        total - o - pr,
        s.iterations
    );
}

