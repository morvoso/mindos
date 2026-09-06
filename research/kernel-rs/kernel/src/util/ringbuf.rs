//! Fixed-capacity byte ring buffer.

pub struct RingBuf<const N: usize> {
    buf: [u8; N],
    head: usize,
    tail: usize,
}

impl<const N: usize> RingBuf<N> {
    pub const fn new() -> Self {
        RingBuf { buf: [0; N], head: 0, tail: 0 }
    }
    pub fn len(&self) -> usize {
        self.head.wrapping_sub(self.tail)
    }
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }
    pub fn is_full(&self) -> bool {
        self.len() == N
    }
    pub fn capacity(&self) -> usize {
        N
    }
    pub fn space(&self) -> usize {
        N - self.len()
    }
    pub fn push(&mut self, b: u8) -> bool {
        if self.is_full() {
            return false;
        }
        self.buf[self.head % N] = b;
        self.head = self.head.wrapping_add(1);
        true
    }
    pub fn pop(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let b = self.buf[self.tail % N];
        self.tail = self.tail.wrapping_add(1);
        Some(b)
    }
    pub fn peek(&self) -> Option<u8> {
        if self.is_empty() {
            None
        } else {
            Some(self.buf[self.tail % N])
        }
    }
    pub fn write(&mut self, data: &[u8]) -> usize {
        let mut n = 0;
        for &b in data {
            if !self.push(b) {
                break;
            }
            n += 1;
        }
        n
    }
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0;
        while n < out.len() {
            match self.pop() {
                Some(b) => {
                    out[n] = b;
                    n += 1;
                }
                None => break,
            }
        }
        n
    }
    pub fn clear(&mut self) {
        self.tail = self.head;
    }
    /// Remove the most recently pushed byte (for line editing).
    pub fn unpush(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        self.head = self.head.wrapping_sub(1);
        Some(self.buf[self.head % N])
    }
}
