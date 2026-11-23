use knorke_types::{Parallel, chunk_count};
use std::num::NonZeroUsize;

/// Splits work across freshly spawned scoped `std` threads (static partitioning).
///
/// No dependencies, but every call spawns `threads - 1` OS threads (tens of µs each on
/// Windows). Prefer [`Rayon`](crate::Rayon) in a frame loop.
#[derive(Clone, Copy, Debug)]
pub struct ScopedThreads {
    threads: NonZeroUsize,
}

impl ScopedThreads {
    /// At most `threads` threads (including the caller's).
    pub fn new(threads: NonZeroUsize) -> Self {
        Self { threads }
    }

    /// As many threads as the OS reports cores.
    pub fn available() -> Self {
        Self::new(std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN))
    }
}

impl Parallel for ScopedThreads {
    fn threads(&self) -> usize {
        self.threads.get()
    }

    fn for_each_chunk_mut<T: Send>(
        &self,
        items: &mut [T],
        chunk_len: usize,
        f: impl Fn(usize, &mut [T]) + Sync,
    ) {
        let chunks = chunk_count(items.len(), chunk_len);
        let workers = self.threads.get().min(chunks);
        if workers <= 1 {
            for (i, c) in items.chunks_mut(chunk_len).enumerate() {
                f(i, c);
            }
            return;
        }
        // Contiguous groups of whole chunks per worker; the caller runs the last group.
        let per_worker = chunks.div_ceil(workers);
        let f = &f;
        std::thread::scope(|scope| {
            let mut rest = items;
            let mut first_chunk = 0;
            while first_chunk < chunks {
                let n_chunks = per_worker.min(chunks - first_chunk);
                let n_items = (n_chunks * chunk_len).min(rest.len());
                let (group, tail) = rest.split_at_mut(n_items);
                rest = tail;
                let base = first_chunk;
                first_chunk += n_chunks;
                let run = move || {
                    let group = group;
                    for (i, c) in group.chunks_mut(chunk_len).enumerate() {
                        f(base + i, c);
                    }
                };
                if first_chunk < chunks {
                    scope.spawn(run);
                } else {
                    run();
                }
            }
        });
    }

    fn join<RA: Send, RB: Send>(
        &self,
        a: impl FnOnce() -> RA + Send,
        b: impl FnOnce() -> RB + Send,
    ) -> (RA, RB) {
        if self.threads.get() == 1 {
            return (a(), b());
        }
        std::thread::scope(|scope| {
            let hb = scope.spawn(b);
            let ra = a();
            (
                ra,
                hb.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
            )
        })
    }
}
