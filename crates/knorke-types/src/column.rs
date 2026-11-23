//! Borrowed views of data owned by someone else.
//!
//! The rule for data exchange between Knorke modules (ADR 0006): a module that needs
//! many values from its caller takes a [`Column`] (or a plain slice) and never asks the
//! caller to build a `Vec` for it. A host whose positions live inside its own structs
//! hands them over with [`project`]: no copy, and after monomorphisation the access is
//! the same machine code as reading the field directly.

/// Read-only random access to `len()` values of `T` owned by someone else.
///
/// Idiom for module APIs: take the column **by value**, `positions: impl Column<Pos>`.
/// References implement `Column` too, so callers pass `&slice[..]`, `&vec`, or a
/// [`project`]ion, and slices (unsized) need no `?Sized` bound in the signature. Same for
/// [`ColumnMut`]: `out: impl ColumnMut<T>`, called with `&mut buffer[..]`.
pub trait Column<T: ?Sized> {
    /// Number of values.
    fn len(&self) -> usize;

    /// The value at `index`, or `None` if out of range.
    fn get(&self, index: usize) -> Option<&T>;

    /// `true` if there are no values.
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates over all values in index order. Implementations backed by a slice override
    /// this with the slice iterator; references forward to it.
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        (0..self.len()).filter_map(|i| self.get(i))
    }
}

/// Read-write random access to values owned by someone else, e.g. to write results back
/// into the host's structs.
pub trait ColumnMut<T: ?Sized>: Column<T> {
    /// The value at `index`, or `None` if out of range.
    fn get_mut(&mut self, index: usize) -> Option<&mut T>;
}

impl<T> Column<T> for [T] {
    #[inline]
    fn len(&self) -> usize {
        <[T]>::len(self)
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        <[T]>::get(self, index)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        <[T]>::iter(self)
    }
}

impl<T> ColumnMut<T> for [T] {
    #[inline]
    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        <[T]>::get_mut(self, index)
    }
}

impl<T, const N: usize> Column<T> for [T; N] {
    #[inline]
    fn len(&self) -> usize {
        N
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        self.as_slice().iter()
    }
}

impl<T> Column<T> for alloc::vec::Vec<T> {
    #[inline]
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        self.as_slice().get(index)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        self.as_slice().iter()
    }
}

impl<T: ?Sized, C: Column<T> + ?Sized> Column<T> for &C {
    #[inline]
    fn len(&self) -> usize {
        (**self).len()
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        (**self).get(index)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        (**self).iter()
    }
}

impl<T: ?Sized, C: Column<T> + ?Sized> Column<T> for &mut C {
    #[inline]
    fn len(&self) -> usize {
        (**self).len()
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        (**self).get(index)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        (**self).iter()
    }
}

impl<T: ?Sized, C: ColumnMut<T> + ?Sized> ColumnMut<T> for &mut C {
    #[inline]
    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        (**self).get_mut(index)
    }
}

/// One field of every element of a borrowed slice, as a [`Column`]. Built by [`project`].
pub struct Project<'a, S, F> {
    items: &'a [S],
    field: F,
}

/// Views one field of each element of `items` without copying.
///
/// ```
/// use knorke_types::{Column, project};
/// struct Npc { pos: [i32; 2], hp: u16 }
/// let npcs = [Npc { pos: [1, 2], hp: 9 }, Npc { pos: [3, 4], hp: 7 }];
/// let positions = project(&npcs, |n| &n.pos);
/// assert_eq!(positions.get(1), Some(&[3, 4]));
/// assert!(std::ptr::eq(positions.get(0).unwrap(), &npcs[0].pos));
/// ```
#[inline]
pub fn project<'a, S, T: ?Sized + 'a, F: Fn(&'a S) -> &'a T>(
    items: &'a [S],
    field: F,
) -> Project<'a, S, F> {
    Project { items, field }
}

impl<'a, S, T: ?Sized + 'a, F: Fn(&'a S) -> &'a T> Column<T> for Project<'a, S, F> {
    #[inline]
    fn len(&self) -> usize {
        self.items.len()
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        let items: &'a [S] = self.items;
        items.get(index).map(&self.field)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        let items: &'a [S] = self.items;
        let field = &self.field;
        items.iter().map(move |s| -> &'s T { field(s) })
    }
}

/// One field of every element of a mutably borrowed slice. Built by [`project_mut`].
pub struct ProjectMut<'a, S, R, W> {
    items: &'a mut [S],
    read: R,
    write: W,
}

/// Mutable variant of [`project`]: `read` and `write` must select the same field.
///
/// ```
/// use knorke_types::{ColumnMut, project_mut};
/// struct Npc { velocity: [f32; 2] }
/// let mut npcs = [Npc { velocity: [0.0; 2] }];
/// let mut v = project_mut(&mut npcs, |n| &n.velocity, |n| &mut n.velocity);
/// *v.get_mut(0).unwrap() = [1.0, 0.5];
/// assert_eq!(npcs[0].velocity, [1.0, 0.5]);
/// ```
#[inline]
pub fn project_mut<S, T: ?Sized, R, W>(
    items: &mut [S],
    read: R,
    write: W,
) -> ProjectMut<'_, S, R, W>
where
    R: Fn(&S) -> &T,
    W: Fn(&mut S) -> &mut T,
{
    ProjectMut { items, read, write }
}

impl<S, T: ?Sized, R: Fn(&S) -> &T, W> Column<T> for ProjectMut<'_, S, R, W> {
    #[inline]
    fn len(&self) -> usize {
        self.items.len()
    }
    #[inline]
    fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index).map(&self.read)
    }
    #[inline]
    fn iter<'s>(&'s self) -> impl Iterator<Item = &'s T>
    where
        T: 's,
    {
        self.items.iter().map(&self.read)
    }
}

impl<S, T: ?Sized, R: Fn(&S) -> &T, W: Fn(&mut S) -> &mut T> ColumnMut<T>
    for ProjectMut<'_, S, R, W>
{
    #[inline]
    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index).map(&self.write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Host {
        pos: (i32, i32),
        _other: [u8; 3],
    }

    fn sum_x(c: &(impl Column<(i32, i32)> + ?Sized)) -> i32 {
        c.iter().map(|p| p.0).sum()
    }

    #[test]
    fn slices_vecs_arrays_and_projections_are_columns() {
        let raw = [(1, 0), (2, 0)];
        let hosts = [
            Host {
                pos: (1, 0),
                _other: [0; 3],
            },
            Host {
                pos: (2, 0),
                _other: [0; 3],
            },
        ];
        assert_eq!(sum_x(&raw), 3);
        assert_eq!(sum_x(&raw[..]), 3);
        assert_eq!(sum_x(&alloc::vec::Vec::from(raw)), 3);
        assert_eq!(sum_x(&project(&hosts, |h| &h.pos)), 3);
    }

    /// A column whose `get` counts calls and whose `iter` does not use `get`.
    struct Counted<'a>(&'a [u8], core::cell::Cell<u32>);
    impl Column<u8> for Counted<'_> {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn get(&self, i: usize) -> Option<&u8> {
            self.1.set(self.1.get() + 1);
            self.0.get(i)
        }
        fn iter<'s>(&'s self) -> impl Iterator<Item = &'s u8>
        where
            u8: 's,
        {
            self.0.iter()
        }
    }

    #[test]
    fn references_forward_the_fast_iterator() {
        fn total(c: impl Column<u8>) -> u32 {
            c.iter().map(|&v| v as u32).sum()
        }
        let mut c = Counted(&[1, 2, 3], core::cell::Cell::new(0));
        assert_eq!(total(&c), 6);
        assert_eq!(total(&mut c), 6);
        assert_eq!(c.1.get(), 0, "`&C` fell back to index-by-index access");
        assert_eq!(total([4u8, 5]), 9);
        let mut hosts = [Host {
            pos: (7, 0),
            _other: [0; 3],
        }];
        let col = project_mut(&mut hosts, |h| &h.pos, |h| &mut h.pos);
        assert_eq!(sum_x(&col), 7);
    }

    #[test]
    fn projection_does_not_copy() {
        let hosts = [Host {
            pos: (5, 6),
            _other: [0; 3],
        }];
        let col = project(&hosts, |h| &h.pos);
        assert!(core::ptr::eq(col.get(0).unwrap(), &hosts[0].pos));
        assert_eq!(col.get(1), None);
    }

    #[test]
    fn projection_mut_writes_through() {
        let mut hosts = [
            Host {
                pos: (0, 0),
                _other: [0; 3],
            },
            Host {
                pos: (0, 0),
                _other: [0; 3],
            },
        ];
        let mut col = project_mut(&mut hosts, |h| &h.pos, |h| &mut h.pos);
        for i in 0..col.len() {
            col.get_mut(i).unwrap().0 = i as i32 + 10;
        }
        assert_eq!(sum_x(&col), 21);
        assert_eq!(hosts[1].pos, (11, 0));
    }
}
