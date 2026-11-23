use knorke_types::Parallel;
use rayon::prelude::*;

/// Work-stealing executor on a rayon thread pool: the global one or a dedicated pool.
///
/// A dedicated pool keeps engine work off threads that other libraries use and lets the
/// host reserve cores (e.g. one for the render thread).
#[derive(Debug, Default)]
pub struct Rayon {
    pool: Option<rayon::ThreadPool>,
}

impl Rayon {
    /// Uses rayon's global pool.
    pub fn global() -> Self {
        Self { pool: None }
    }

    /// Creates a dedicated pool with `threads` worker threads named `knorke-worker-N`.
    pub fn with_threads(threads: usize) -> Result<Self, rayon::ThreadPoolBuildError> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("knorke-worker-{i}"))
            .build()?;
        Ok(Self { pool: Some(pool) })
    }

    /// Runs `op` on a pool thread and waits for it. Use it to run a whole frame inside the
    /// pool: parallel calls made from a pool thread use the thread's local queue, while each
    /// call from outside goes through rayon's global injector (which allocates one block per
    /// 63 injected jobs).
    pub fn install<R: Send>(&self, op: impl FnOnce() -> R + Send) -> R {
        match &self.pool {
            Some(p) => p.install(op),
            None => op(),
        }
    }
}

impl Parallel for Rayon {
    fn threads(&self) -> usize {
        match &self.pool {
            Some(p) => p.current_num_threads(),
            None => rayon::current_num_threads(),
        }
    }

    fn for_each_chunk_mut<T: Send>(
        &self,
        items: &mut [T],
        chunk_len: usize,
        f: impl Fn(usize, &mut [T]) + Sync,
    ) {
        assert!(chunk_len > 0, "chunk_len must be > 0");
        let f = &f;
        self.install(move || {
            items
                .par_chunks_mut(chunk_len)
                .enumerate()
                .for_each(move |(i, c)| f(i, c));
        });
    }

    fn join<RA: Send, RB: Send>(
        &self,
        a: impl FnOnce() -> RA + Send,
        b: impl FnOnce() -> RB + Send,
    ) -> (RA, RB) {
        self.install(move || rayon::join(a, b))
    }
}
