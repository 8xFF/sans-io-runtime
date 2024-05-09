//! A bit vector implementation. which is used inside task switcher

pub struct BitVec {
    bytes: Vec<u8>,
    len: usize,
}

impl BitVec {
    pub fn news(len: usize) -> Self {
        Self {
            bytes: vec![0; len / 8 + 1].to_vec(),
            len,
        }
    }

    pub fn set_len(&mut self, len: usize) {
        if self.len > self.bytes.len() * 8 {
            self.bytes.resize(len / 8 + 1, 0);
        }
        self.len = len;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn get_bit(&self, index: usize) -> bool {
        assert!(self.len > index, "index out of bounds");
        let byte_index = index / 8;
        let bit_index = index % 8;
        self.bytes[byte_index] & (1 << bit_index) != 0
    }

    pub fn set_bit(&mut self, index: usize, value: bool) {
        assert!(self.len > index, "index out of bounds");
        let byte_index = index / 8;
        let bit_index = index % 8;
        if value {
            self.bytes[byte_index] |= 1 << bit_index;
        } else {
            self.bytes[byte_index] &= !(1 << bit_index);
        }
    }

    pub fn first_set_index(&self) -> Option<usize> {
        for (byte_index, &byte) in self.bytes.iter().enumerate() {
            if byte != 0 {
                for bit_index in 0..8 {
                    let index = byte_index * 8 + bit_index;
                    if index >= self.len {
                        return None;
                    }
                    if byte & (1 << bit_index) != 0 {
                        return Some(index);
                    }
                }
            }
        }
        None
    }

    pub fn set_all(&mut self, value: bool) {
        self.bytes.fill(if value { 0xff } else { 0 });
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn simple_work() {
        let mut bit_vec = super::BitVec::news(8);
        assert_eq!(bit_vec.first_set_index(), None);
        bit_vec.set_bit(0, true);
        assert_eq!(bit_vec.first_set_index(), Some(0));
        bit_vec.set_bit(0, false);
        assert_eq!(bit_vec.first_set_index(), None);
        bit_vec.set_bit(7, true);
        assert_eq!(bit_vec.get_bit(7), true);
        assert_eq!(bit_vec.first_set_index(), Some(7));
        bit_vec.set_bit(7, false);
        assert_eq!(bit_vec.first_set_index(), None);
        bit_vec.set_bit(0, true);
        bit_vec.set_bit(7, true);
        assert_eq!(bit_vec.first_set_index(), Some(0));
        bit_vec.set_all(true);
        assert_eq!(bit_vec.first_set_index(), Some(0));
        bit_vec.set_all(false);
        assert_eq!(bit_vec.first_set_index(), None);
    }
}
