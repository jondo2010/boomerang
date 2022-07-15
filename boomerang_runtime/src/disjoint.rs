use std::{borrow::Borrow, marker::PhantomData};

pub struct DisjointChunked<'a, T: 'a + Send, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    data: Box<[T]>,
    offset: usize,
    indices: I,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: 'a + Send, I, K> DisjointChunked<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    fn new(data: Box<[T]>, indices: I) -> Self {
        Self {
            data,
            offset: 0,
            indices,
            phantom: PhantomData,
        }
    }
}

impl<'a, T: 'a + Send, I, K> Iterator for DisjointChunked<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        let width = self.indices.next();
        let remaining = self.data.len() - self.offset;
        match (width, remaining) {
            (Some(width), remaining) if remaining >= *width.borrow() => {
                let offset = self.offset as isize;
                self.offset += *width.borrow();
                Some(unsafe {
                    std::slice::from_raw_parts(self.data.as_ptr().offset(offset), *width.borrow())
                })
            }
            _ => None,
        }
    }
}

pub struct DisjointChunkedMut<'a, T: Send, I, K>(DisjointChunked<'a, T, I, K>)
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send;

impl<'a, T: 'a + Send, I, K> DisjointChunkedMut<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    fn new(data: Box<[T]>, indices: I) -> Self {
        Self(DisjointChunked {
            data,
            offset: 0,
            indices,
            phantom: PhantomData,
        })
    }
}

impl<'a, T: 'a + Send, I, K> Iterator for DisjointChunkedMut<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    type Item = &'a mut [T];

    fn next(&mut self) -> Option<Self::Item> {
        let slf = &mut self.0;
        let width = slf.indices.next();
        let remaining = slf.data.len() - slf.offset;
        match (width, remaining) {
            (Some(width), remaining) if remaining >= *width.borrow() => {
                let offset = slf.offset as isize;
                slf.offset += *width.borrow();
                Some(unsafe {
                    std::slice::from_raw_parts_mut(
                        slf.data.as_mut_ptr().offset(offset),
                        *width.borrow(),
                    )
                })
            }
            _ => None,
        }
    }
}

pub(crate) unsafe fn disjoint_unchecked<'sm, K, KV, V, I1, I2>(
    sm: &'sm mut slotmap::SlotMap<K, KV>,
    keys: I1,
    mut_keys: I2,
) -> (Box<[&'sm V]>, Box<[&'sm mut V]>)
where
    K: slotmap::Key,
    KV: AsRef<V> + AsMut<V>,
    V: ?Sized,
    I1: IntoIterator<Item = K>,
    I2: IntoIterator<Item = K>,
    I1::IntoIter: ExactSizeIterator,
    I2::IntoIter: ExactSizeIterator,
{
    let keys = keys.into_iter();
    let mut_keys = mut_keys.into_iter();

    let mut optrs = Box::<[*const V]>::new_uninit_slice(mut_keys.len());
    for (ptr, key) in optrs.iter_mut().zip(mut_keys) {
        ptr.as_mut_ptr().write(sm.get_unchecked_mut(key).as_mut());
    }
    let oraw = core::mem::transmute_copy(&Box::into_raw(optrs.assume_init()));

    let mut iptrs = Box::<[*const V]>::new_uninit_slice(keys.len());
    for (ptr, key) in iptrs.iter_mut().zip(keys) {
        ptr.as_mut_ptr().write(sm.get_unchecked(key).as_ref())
    }
    let iraw = core::mem::transmute_copy(&Box::into_raw(iptrs.assume_init()));

    (Box::from_raw(iraw), Box::from_raw(oraw))
}

pub(crate) fn disjoint_unchecked_chunked<'sm, K, KV, T, IOuter1, IOuter2, IInner>(
    sm: &'sm mut slotmap::SlotMap<K, KV>,
    keys: IOuter1,
    keys_mut: IOuter2,
) -> (
    DisjointChunked<'sm, &T, impl Iterator<Item = usize>, usize>,
    DisjointChunkedMut<'sm, &mut T, impl Iterator<Item = usize>, usize>,
)
where
    K: slotmap::Key,
    KV: AsRef<T> + AsMut<T>,
    T: ?Sized + Send + Sync,
    IOuter1: Iterator<Item = IInner> + Clone + Send,
    IOuter2: Iterator<Item = IInner> + Clone + Send,
    IInner: Iterator<Item = &'sm K> + ExactSizeIterator + Clone + Send,
{
    let all_keys = keys.clone().flatten();
    let all_mut_keys = keys_mut.clone().flatten();
    // let (iboxed, oboxed) = unsafe{disjoint_unchecked(sm, all_keys, all_mut_keys)};

    let chunks = {
        let keys_indices = keys.clone().map(|inner| inner.len());
        let mut iptrs = Box::<[*const T]>::new_uninit_slice(keys_indices.clone().sum());
        let boxed = unsafe {
            for (&key, ptr) in all_keys.clone().zip(iptrs.iter_mut()) {
                ptr.as_mut_ptr().write(sm.get_unchecked(key).as_ref())
            }
            let iraw = core::mem::transmute_copy(&Box::into_raw(iptrs.assume_init()));
            Box::<[&T]>::from_raw(iraw)
        };
        DisjointChunked::new(boxed, keys_indices)
    };

    let chunks_mut = {
        let keys_indices = keys_mut.clone().map(|inner| inner.len());
        let mut iptrs = Box::<[*const T]>::new_uninit_slice(keys_indices.clone().sum());
        let boxed = unsafe {
            for (&key, ptr) in all_mut_keys.clone().zip(iptrs.iter_mut()) {
                ptr.as_mut_ptr().write(sm.get_unchecked_mut(key).as_mut())
            }
            let iraw = core::mem::transmute_copy(&Box::into_raw(iptrs.assume_init()));
            Box::<[&mut T]>::from_raw(iraw)
        };
        DisjointChunkedMut::new(boxed, keys_indices)
    };
    (chunks, chunks_mut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::{DefaultKey, SlotMap};

    fn make_slotmap<const N: usize>() -> (SlotMap<DefaultKey, Box<usize>>, [DefaultKey; N]) {
        let mut sm = SlotMap::new();
        let mut keys = [DefaultKey::default(); N];
        for (i, k) in keys.iter_mut().enumerate() {
            *k = sm.insert(Box::new(i));
        }
        (sm, keys)
    }

    #[test]
    fn test_disjoint_chunked() {
        let dat = vec![1, 2, 3, 4, 5].into_boxed_slice();
        let ofs = vec![1, 2, 2, 5];
        let mut dj = DisjointChunked::new(dat, ofs.iter());
        assert_eq!(dj.next(), Some(&[1] as &[_]));
        assert_eq!(dj.next(), Some(&[2, 3] as &[_]));
        assert_eq!(dj.next(), Some(&[4, 5] as &[_]));
        assert_eq!(dj.next(), None);
    }

    #[test]
    fn test_disjoint_chunked_mut() {
        let (mut sm, keys) = make_slotmap::<5>();
        let vv = sm.get_disjoint_mut([keys[0], keys[1]]).unwrap().into();
        let ofs = vec![1, 2, 2, 5];
        let mut dj = DisjointChunkedMut::new(vv, ofs.iter());
        let x = dj.next().unwrap();
        assert_eq!(x.len(), 1);
        *x[0].as_mut() += 1usize;
        assert_eq!(sm.get(keys[0]).unwrap().as_ref(), &1);
    }

    #[test]
    fn test_disjoint_unchecked() {
        let (mut sm, keys) = make_slotmap::<5>();
        {
            let i_keys = keys[0..=1].iter().cloned();
            let o_keys = keys[2..=3].iter().cloned();
            let (i, o) = unsafe { disjoint_unchecked(&mut sm, i_keys, o_keys) };
            assert_eq!(i.len(), 2);
            assert_eq!(*i[0], 0);
            assert_eq!(*i[1], 1);
            assert_eq!(o.len(), 2);
            assert_eq!(*o[0], 2);
            assert_eq!(*o[1], 3);
        }
    }

    #[test]
    fn test_disjoint_chunked2() {
        let (mut sm, keys) = make_slotmap::<5>();
        let keys = vec![&keys[0..=1], &keys[2..=3]];
        let (_i, mut o) =
            disjoint_unchecked_chunked(&mut sm, std::iter::empty(), keys.iter().map(|x| x.iter()));

        let _bar = o.next().unwrap();
        // dbg!(bar);
        let _bar = o.next().unwrap();
        // dbg!(bar);
    }
}
