use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone)]
enum BufferViewInner<'a> {
    Ref(&'a [u8]),
}

impl<'a> Deref for BufferViewInner<'a> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(r) => r,
        }
    }
}

impl<'a> From<&'a [u8]> for BufferViewInner<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::Ref(value)
    }
}

#[derive(Debug, Clone)]
enum BufferInner {
    Vec(Vec<u8>),
}

impl BufferInner {
    pub fn view(&self) -> BufferViewInner {
        match self {
            Self::Vec(v) => v.as_slice().into(),
        }
    }

    pub fn allocate_front(&mut self, more: usize) {
        match self {
            Self::Vec(v) => {
                let mut vec2 = vec![0; more + v.len()];
                vec2[more..].copy_from_slice(v);
                let _ = std::mem::replace(v, vec2);
            }
        }
    }

    pub fn allocate_back(&mut self, more: usize) {
        match self {
            Self::Vec(v) => {
                v.append(&mut vec![0; more]);
            }
        }
    }
}

impl Deref for BufferInner {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Vec(v) => v,
        }
    }
}

impl DerefMut for BufferInner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Vec(v) => v,
        }
    }
}

impl From<Vec<u8>> for BufferInner {
    fn from(value: Vec<u8>) -> Self {
        Self::Vec(value)
    }
}

#[derive(Debug, Clone)]
pub struct BufferView<'a> {
    buf: BufferViewInner<'a>,
    pub range: std::ops::Range<usize>,
}

impl<'a> BufferView<'a> {
    pub fn view(&self, range: std::ops::Range<usize>) -> Option<BufferView> {
        if self.range.end - self.range.start >= range.end {
            Some(BufferView {
                buf: self.buf.clone(),
                range: (self.range.start + range.start..self.range.start + range.end),
            })
        } else {
            None
        }
    }

    pub fn pop_back(&mut self, len: usize) -> Option<BufferView> {
        if self.range.end - self.range.start >= len {
            self.range.end -= len;
            Some(BufferView {
                buf: self.buf.clone(),
                range: (self.range.end..self.range.end + len),
            })
        } else {
            None
        }
    }

    pub fn pop_front(&mut self, len: usize) -> Option<BufferView> {
        if self.range.end - self.range.start >= len {
            self.range.start += len;
            Some(BufferView {
                buf: self.buf.clone(),
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

impl<'a> Deref for BufferView<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buf.deref()[self.range.clone()]
    }
}

impl<'a> Eq for BufferView<'a> {}

impl<'a> PartialEq for BufferView<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<'a> From<&'a [u8]> for BufferView<'a> {
    fn from(value: &'a [u8]) -> Self {
        BufferView {
            buf: value.into(),
            range: (0..value.len()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Buffer {
    buf: BufferInner,
    range: std::ops::Range<usize>,
}

impl Buffer {
    /// Create a buffer with pre-defined size. The left path is useful when we
    /// working with networking protocol. Sometime we need to append data to front of buffer
    /// the left-padding with avoiding vector need to re-create
    pub fn new(left: usize, main: usize) -> Buffer {
        let mut v = Vec::with_capacity(left + main);
        unsafe {
            v.set_len(left + main);
        }
        Buffer {
            buf: v.into(),
            range: (left..left),
        }
    }

    /// Create a buffer mut with append some bytes at first and some bytes at end
    pub fn build(data: &[u8], more_left: usize, more_right: usize) -> Buffer {
        let mut v = vec![0; more_left + data.len() + more_right];
        v[more_left..more_left + data.len()].copy_from_slice(data);
        Buffer {
            buf: v.into(),
            range: (more_left..more_left + data.len()),
        }
    }

    /// Create a new buffer from a vec, we can manually set the start and end of the buffer.
    pub fn from_vec_raw(data: Vec<u8>, range: std::ops::Range<usize>) -> Buffer {
        assert!(range.end <= data.len());
        Buffer {
            buf: data.into(),
            range,
        }
    }

    /// Getting front buffer for writing
    pub fn front_mut(&mut self, len: usize) -> &mut [u8] {
        self.ensure_front(len);
        &mut self.buf.deref_mut()[self.range.start - len..self.range.start]
    }

    /// Getting remain buffer for writing, this useful when using with UDP Socket
    pub fn remain_mut(&mut self) -> &mut [u8] {
        &mut self.buf.deref_mut()[self.range.start..]
    }

    /// Getting back buffer for writing
    pub fn back_mut(&mut self, len: usize) -> &mut [u8] {
        self.ensure_back(len);
        &mut self.buf.deref_mut()[self.range.end..self.range.end + len]
    }

    /// Reverse the front for at least `len` bytes at front
    pub fn ensure_front(&mut self, more: usize) {
        if self.range.start < more {
            self.buf.allocate_front(more - self.range.start);
            self.range.start += more;
            self.range.end += more;
        }
    }

    /// Reverse the buffer for at least `len` bytes at back.
    pub fn ensure_back(&mut self, more: usize) {
        assert!(self.buf.len() >= self.range.end);
        let remain = self.buf.len() - self.range.end;
        if remain < more {
            self.buf.allocate_back(more - remain);
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

    pub fn push_front(&mut self, data: &[u8]) {
        self.ensure_front(data.len());
        self.range.start -= data.len();
        self.buf.deref_mut()[self.range.start..self.range.start + data.len()].copy_from_slice(data);
    }

    pub fn push_back(&mut self, data: &[u8]) {
        self.ensure_back(data.len());
        self.buf.deref_mut()[self.range.end..self.range.end + data.len()].copy_from_slice(data);
        self.range.end += data.len();
    }

    pub fn pop_back(&mut self, len: usize) -> Option<BufferView<'_>> {
        if self.range.end - self.range.start >= len {
            self.range.end -= len;
            Some(BufferView {
                buf: self.buf.view(),
                range: (self.range.end..self.range.end + len),
            })
        } else {
            None
        }
    }

    pub fn pop_front(&mut self, len: usize) -> Option<BufferView<'_>> {
        if self.range.end - self.range.start >= len {
            self.range.start += len;
            Some(BufferView {
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

    pub fn move_back_right(&mut self, len: usize) -> Option<()> {
        if self.range.end + len > self.buf.len() {
            return None;
        }
        self.range.end += len;
        Some(())
    }

    pub fn move_back_left(&mut self, len: usize) -> Option<()> {
        if self.range.end < len || self.range.end - len < self.range.start {
            return None;
        }
        self.range.end -= len;
        Some(())
    }
}

impl From<Vec<u8>> for Buffer {
    fn from(value: Vec<u8>) -> Self {
        Buffer {
            range: (0..value.len()),
            buf: value.into(),
        }
    }
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.buf.deref()[self.range.start..self.range.end]
    }
}

impl Eq for Buffer {}

impl PartialEq for Buffer {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buf.deref_mut()[self.range.start..self.range.end]
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Deref, vec};

    use super::{Buffer, BufferInner, BufferView};

    #[test]
    fn simple_buffer_view() {
        let data = vec![1, 2, 3, 4, 5, 6];
        let mut buf: BufferView = (data.as_slice()).into();
        assert_eq!(buf.len(), 6);
        assert_eq!(buf.pop_back(2).expect("").deref(), &[5, 6]);
        assert_eq!(buf.pop_front(2).expect("").deref(), &[1, 2]);
        assert_eq!(buf.deref(), &[3, 4]);
    }

    #[test]
    fn simple_buffer_mut() {
        let mut buf = Buffer::build(&[1, 2, 3, 4, 5, 6], 4, 4);
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
    fn buffer_expand() {
        let mut buf = Buffer::from(vec![1, 2, 3, 4]);
        buf.ensure_front(2);
        buf.ensure_back(2);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.front_mut(2), &[0, 0]);
        assert_eq!(buf.back_mut(2), &[0, 0]);

        buf.push_front(&[8, 8]);
        buf.push_back(&[9, 9]);

        assert_eq!(buf.deref(), &[8, 8, 1, 2, 3, 4, 9, 9]);
    }

    #[test]
    fn buffer_inner_expand() {
        let mut buf = BufferInner::from(vec![1, 2, 3, 4]);
        buf.allocate_front(2);
        assert_eq!(buf.deref(), &[0, 0, 1, 2, 3, 4]);

        let mut buf = BufferInner::Vec(vec![1, 2, 3, 4]);
        buf.allocate_back(2);
        assert_eq!(buf.len(), 6);
        assert_eq!(buf.deref(), &[1, 2, 3, 4, 0, 0]);
    }
}
