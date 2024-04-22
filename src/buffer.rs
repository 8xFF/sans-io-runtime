use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone)]
enum BufferInner<'a> {
    Ref(&'a [u8]),
    Vec(Vec<u8>),
}

impl<'a> BufferInner<'a> {
    pub fn view(&self) -> BufferInner {
        match self {
            Self::Ref(r) => BufferInner::Ref(r),
            Self::Vec(v) => BufferInner::Ref(v),
        }
    }

    pub fn owned(self) -> BufferInner<'static> {
        match self {
            Self::Ref(r) => BufferInner::Vec(r.to_vec()),
            Self::Vec(v) => BufferInner::Vec(v),
        }
    }

    pub fn owned_mut(self) -> BufferMutInner<'static> {
        match self {
            Self::Ref(r) => BufferMutInner::Vec(r.to_vec()),
            Self::Vec(v) => BufferMutInner::Vec(v),
        }
    }

    pub fn clone_mut(&self) -> BufferMutInner<'static> {
        match self {
            Self::Ref(r) => BufferMutInner::Vec(r.to_vec()),
            Self::Vec(v) => BufferMutInner::Vec(v.to_vec()),
        }
    }
}

impl<'a> Deref for BufferInner<'a> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(r) => r,
            Self::Vec(v) => v,
        }
    }
}

impl<'a> From<&'a [u8]> for BufferInner<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::Ref(value)
    }
}

impl<'a> From<Vec<u8>> for BufferInner<'a> {
    fn from(value: Vec<u8>) -> Self {
        Self::Vec(value)
    }
}

#[derive(Debug)]
enum BufferMutInner<'a> {
    Ref(&'a mut [u8]),
    Vec(Vec<u8>),
}

impl<'a> BufferMutInner<'a> {
    pub fn owned(self) -> BufferMutInner<'static> {
        match self {
            Self::Ref(r) => BufferMutInner::Vec(r.to_vec()),
            Self::Vec(v) => BufferMutInner::Vec(v),
        }
    }

    pub fn freeze(self) -> BufferInner<'a> {
        match self {
            Self::Ref(r) => BufferInner::Ref(r),
            Self::Vec(v) => BufferInner::Vec(v),
        }
    }

    pub fn copy_readonly(&self) -> BufferInner<'static> {
        match self {
            Self::Ref(r) => BufferInner::Vec(r.to_vec()),
            Self::Vec(v) => BufferInner::Vec(v.clone()),
        }
    }

    pub fn view(&self) -> BufferInner {
        match self {
            Self::Ref(r) => BufferInner::Ref(r),
            Self::Vec(v) => BufferInner::Ref(v),
        }
    }

    pub fn allocate_more(&mut self, more: usize) {
        match self {
            Self::Ref(r) => {
                let mut v = vec![0; r.len() + more];
                v[0..r.len()].copy_from_slice(r);
                *self = Self::Vec(v);
            }
            Self::Vec(v) => {
                v.append(&mut vec![0; more]);
            }
        }
    }

    pub fn slice_mut<'b>(&'a mut self) -> &'b mut [u8]
    where
        'a: 'b,
    {
        match self {
            Self::Ref(r) => r,
            Self::Vec(v) => v,
        }
    }
}

impl<'a> Deref for BufferMutInner<'a> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(r) => r,
            Self::Vec(v) => v,
        }
    }
}

impl<'a> DerefMut for BufferMutInner<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Ref(r) => r,
            Self::Vec(v) => v,
        }
    }
}

impl<'a> From<&'a mut [u8]> for BufferMutInner<'a> {
    fn from(value: &'a mut [u8]) -> Self {
        Self::Ref(value)
    }
}

impl<'a> From<Vec<u8>> for BufferMutInner<'a> {
    fn from(value: Vec<u8>) -> Self {
        Self::Vec(value)
    }
}

#[derive(Debug, Clone)]
pub struct Buffer<'a> {
    buf: BufferInner<'a>,
    pub range: std::ops::Range<usize>,
}

impl<'a> Buffer<'a> {
    pub fn owned(self) -> Buffer<'static> {
        Buffer {
            buf: self.buf.owned(),
            range: self.range,
        }
    }

    pub fn owned_mut(self) -> BufferMut<'static> {
        BufferMut {
            buf: self.buf.owned_mut(),
            range: self.range,
        }
    }

    pub fn clone_mut(&self) -> BufferMut<'static> {
        BufferMut {
            buf: self.buf.clone_mut(),
            range: self.range.clone(),
        }
    }

    pub fn view(&self, range: std::ops::Range<usize>) -> Option<Buffer> {
        if self.range.end - self.range.start >= range.end {
            Some(Buffer {
                buf: self.buf.view(),
                range: (self.range.start + range.start..self.range.start + range.end),
            })
        } else {
            None
        }
    }

    pub fn pop_back(&mut self, len: usize) -> Option<Buffer> {
        if self.range.end - self.range.start >= len {
            self.range.end -= len;
            Some(Buffer {
                buf: self.buf.view(),
                range: (self.range.end..self.range.end + len),
            })
        } else {
            None
        }
    }

    pub fn pop_front(&mut self, len: usize) -> Option<Buffer> {
        if self.range.end - self.range.start >= len {
            self.range.start += len;
            Some(Buffer {
                buf: self.buf.view(),
                range: (self.range.start - len..self.range.start),
            })
        } else {
            None
        }
    }

    pub fn to_slice2(&self) -> &'_ [u8] {
        &self.buf.deref()[self.range.clone()]
    }
}

impl<'a> Deref for Buffer<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buf.deref()[self.range.clone()]
    }
}

impl<'a> From<&'a [u8]> for Buffer<'a> {
    fn from(value: &'a [u8]) -> Self {
        Buffer {
            buf: value.into(),
            range: (0..value.len()),
        }
    }
}

impl From<Vec<u8>> for Buffer<'_> {
    fn from(value: Vec<u8>) -> Self {
        let len = value.len();
        Buffer {
            buf: value.into(),
            range: (0..len),
        }
    }
}

#[derive(Debug)]
pub struct BufferMut<'a> {
    buf: BufferMutInner<'a>,
    range: std::ops::Range<usize>,
}

impl<'a> BufferMut<'a> {
    pub fn owned(self) -> BufferMut<'static> {
        BufferMut {
            buf: self.buf.owned(),
            range: self.range,
        }
    }

    /// Create a buffer mut with append some bytes at first and some bytes at end
    pub fn build(data: &[u8], more_left: usize, more_right: usize) -> BufferMut<'static> {
        let mut v = vec![0; more_left + data.len() + more_right];
        v[more_left..more_left + data.len()].copy_from_slice(data);
        BufferMut {
            buf: v.into(),
            range: (more_left..more_left + data.len()),
        }
    }

    /// Create a new buffer from a slice, we can manually set the start and end of the buffer.
    pub fn from_slice_raw(data: &'a mut [u8], range: std::ops::Range<usize>) -> BufferMut<'a> {
        assert!(range.end <= data.len());
        BufferMut {
            buf: data.into(),
            range,
        }
    }

    /// Create a new buffer from a vec, we can manually set the start and end of the buffer.
    pub fn from_vec_raw(data: Vec<u8>, range: std::ops::Range<usize>) -> BufferMut<'a> {
        assert!(range.end <= data.len());
        BufferMut {
            buf: data.into(),
            range,
        }
    }

    pub fn freeze(self) -> Buffer<'a> {
        Buffer {
            buf: self.buf.freeze(),
            range: self.range,
        }
    }

    pub fn copy_readonly(&self) -> Buffer<'static> {
        Buffer {
            buf: self.buf.copy_readonly(),
            range: self.range.clone(),
        }
    }

    /// Reverse the buffer for at least `len` bytes at back.
    pub fn ensure_back(&mut self, more: usize) {
        assert!(self.buf.len() >= self.range.end);
        let remain = self.buf.len() - self.range.end;
        if remain < more {
            self.buf.allocate_more(more - remain);
        }
    }

    pub fn truncate(&mut self, len: usize) -> Option<()> {
        if self.range.end - self.range.start >= len {
            self.range.end = self.range.start + len;
            Some(())
        } else {
            None
        }
    }

    pub fn push_back(&mut self, data: &[u8]) {
        self.ensure_back(data.len());
        self.buf.deref_mut()[self.range.end..self.range.end + data.len()].copy_from_slice(data);
        self.range.end += data.len();
    }

    pub fn pop_back(&mut self, len: usize) -> Option<Buffer<'_>> {
        if self.range.end - self.range.start >= len {
            self.range.end -= len;
            Some(Buffer {
                buf: self.buf.view(),
                range: (self.range.end..self.range.end + len),
            })
        } else {
            None
        }
    }

    pub fn pop_front(&mut self, len: usize) -> Option<Buffer<'_>> {
        if self.range.end - self.range.start >= len {
            self.range.start += len;
            Some(Buffer {
                buf: self.buf.view(),
                range: (self.range.start - len..self.range.start),
            })
        } else {
            None
        }
    }

    pub fn move_front_right(&mut self, len: usize) -> Option<()> {
        if self.range.start + len > self.range.end {
            return None;
        }
        self.range.start += len;
        Some(())
    }

    pub fn move_front_left(&mut self, len: usize) -> Option<()> {
        if self.range.start < len {
            return None;
        }
        self.range.start -= len;
        Some(())
    }

    pub fn slice_mut<'b>(&'a mut self) -> &'b mut [u8]
    where
        'a: 'b,
    {
        self.buf.slice_mut()
    }
}

impl<'a> From<&'a mut [u8]> for BufferMut<'a> {
    fn from(value: &'a mut [u8]) -> Self {
        BufferMut {
            range: (0..value.len()),
            buf: value.into(),
        }
    }
}

impl From<Vec<u8>> for BufferMut<'_> {
    fn from(value: Vec<u8>) -> Self {
        BufferMut {
            range: (0..value.len()),
            buf: value.into(),
        }
    }
}

impl Deref for BufferMut<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buf.deref()[self.range.start..self.range.end]
    }
}

impl DerefMut for BufferMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buf.deref_mut()[self.range.start..self.range.end]
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Deref, vec};

    use super::{Buffer, BufferMut, BufferMutInner};

    #[test]
    fn simple_buffer_view() {
        let data = vec![1, 2, 3, 4, 5, 6];
        let mut buf: Buffer = (data.as_slice()).into();
        assert_eq!(buf.len(), 6);
        assert_eq!(buf.pop_back(2).expect("").deref(), &[5, 6]);
        assert_eq!(buf.pop_front(2).expect("").deref(), &[1, 2]);
        assert_eq!(buf.deref(), &[3, 4]);
    }

    #[test]
    fn simple_buffer_mut() {
        let mut buf = BufferMut::build(&[1, 2, 3, 4, 5, 6], 4, 4);
        assert_eq!(buf.deref(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(buf.to_vec(), vec![1, 2, 3, 4, 5, 6]);
        println!("{:?}", buf);
        let res = buf.pop_back(2).expect("");
        println!("{:?}", res);
        assert_eq!(res.deref(), &[5, 6]);
        assert_eq!(res.to_vec(), &[5, 6]);
        assert_eq!(buf.pop_front(2).expect("").deref(), &[1, 2]);
    }

    #[test]
    fn buffer_expend() {
        let mut buf = BufferMutInner::Vec(vec![1, 2, 3, 4]);
        buf.allocate_more(10);
        assert_eq!(buf.len(), 14);

        let mut raw = vec![1, 2, 3, 4];
        let mut buf = BufferMutInner::Ref(&mut raw);
        buf.allocate_more(10);
        assert_eq!(buf.len(), 14);
    }
}
