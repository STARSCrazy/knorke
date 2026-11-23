//! Deferred mutation (ADR 0008).
//!
//! A module that wants to change something it does not own – spawn a projectile, open a
//! door, remove an entity – writes a command instead of reaching into the world. The owner
//! applies all commands at a defined sync point (usually a tick boundary). This is what
//! Bevy's `Commands` and Unity's `EntityCommandBuffer` do.
//!
//! In parallel code every chunk gets its own buffer (see
//! [`Parallel::for_each_chunk_with`](crate::Parallel::for_each_chunk_with)); merging the
//! buffers in chunk order makes the result independent of thread count and timing.

use alloc::vec::Vec;

/// An ordered list of commands of type `C`. Capacity is kept across [`apply`](Self::apply),
/// so a buffer in steady state does not allocate.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandBuffer<C> {
    commands: Vec<C>,
}

impl<C> Default for CommandBuffer<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> CommandBuffer<C> {
    /// Empty buffer.
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Empty buffer with room for `capacity` commands.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            commands: Vec::with_capacity(capacity),
        }
    }

    /// Records a command.
    #[inline]
    pub fn push(&mut self, command: C) {
        self.commands.push(command);
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// `true` if nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Pending commands, oldest first (for inspection and validation before applying).
    pub fn pending(&self) -> &[C] {
        &self.commands
    }

    /// Moves all commands of `other` behind the own ones; `other` keeps its capacity.
    pub fn append(&mut self, other: &mut CommandBuffer<C>) {
        self.commands.append(&mut other.commands);
    }

    /// Applies every pending command in recording order and leaves the buffer empty.
    pub fn apply(&mut self, mut f: impl FnMut(C)) {
        for c in self.commands.drain(..) {
            f(c);
        }
    }

    /// Applies the commands of several buffers in index order (buffer 0 first), e.g. one
    /// buffer per parallel chunk. Deterministic regardless of which thread filled which buffer.
    pub fn apply_all(buffers: &mut [CommandBuffer<C>], mut f: impl FnMut(C)) {
        for b in buffers {
            b.apply(&mut f);
        }
    }

    pub(crate) fn commands_vec_mut(&mut self) -> &mut Vec<C> {
        &mut self.commands
    }

    /// Drops all pending commands.
    pub fn clear(&mut self) {
        self.commands.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Cmd {
        Spawn(u32),
        Despawn(u32),
    }

    #[test]
    fn applies_in_order_and_keeps_capacity() {
        let mut b = CommandBuffer::with_capacity(4);
        b.push(Cmd::Spawn(1));
        b.push(Cmd::Despawn(0));
        assert_eq!(b.pending(), &[Cmd::Spawn(1), Cmd::Despawn(0)]);
        let mut seen = Vec::new();
        b.apply(|c| seen.push(c));
        assert_eq!(seen, [Cmd::Spawn(1), Cmd::Despawn(0)]);
        assert!(b.is_empty());
        assert!(b.commands.capacity() >= 4);
    }

    #[test]
    fn merging_follows_buffer_index() {
        let mut chunks: Vec<CommandBuffer<u32>> = (0..3).map(|_| CommandBuffer::new()).collect();
        chunks[2].push(20);
        chunks[0].push(0);
        chunks[1].push(10);
        chunks[0].push(1);
        let mut seen = Vec::new();
        CommandBuffer::apply_all(&mut chunks, |c| seen.push(c));
        assert_eq!(seen, [0, 1, 10, 20]);
        let mut a = CommandBuffer::new();
        let mut b = CommandBuffer::new();
        a.push(1);
        b.push(2);
        a.append(&mut b);
        assert_eq!((a.pending(), b.len()), (&[1, 2][..], 0));
    }
}
