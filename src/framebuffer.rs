pub struct Framebuffer {
    data: [u8; 256*320/8],
}

impl Framebuffer {
    pub const fn new() -> Self {
        Self {
            data: [0u8; 256*320/8]
        }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, value: bool) {
        let index = x + y * 320;
        let bit = 1u8 << (7 - (index & 0x7));
        if value {
            self.data[index >> 3] |= bit;
        } else {
            self.data[index >> 3] &= !bit;
        }
    }

    pub fn clear(&mut self, value: bool) {
        self.data.fill(if value { 0xFFu8 } else { 0u8 });
    }

    pub fn row_slice(&self, y: usize) -> &[u8] {
        &self.data[((y * 320) >> 3)..(((y + 1) * 320) >> 3)]
    }
}
