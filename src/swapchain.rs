use critical_section::{CriticalSection, Mutex};
use embedded_hal_0_2::can::Frame;
use core::cell::{RefCell, UnsafeCell};

use crate::framebuffer::Framebuffer;

struct SwapchainState {
    read: u32,
    write: u32,
}

pub struct Swapchain {
    buffers: [UnsafeCell<Framebuffer>; 3],
    state: Mutex<RefCell<SwapchainState>>,
}

unsafe impl Send for Swapchain {}
unsafe impl Sync for Swapchain {}

pub struct SwapchainImage<'a> {
    framebuffer: &'a mut Framebuffer,
    index: u32,
    state: &'a Mutex<RefCell<SwapchainState>>,
}

impl SwapchainImage<'_> {
    pub fn submit(self) {
        critical_section::with(|cs| {
            let state = self.state.borrow(cs);
            let mut state_mut = state.borrow_mut();
            let next_write = match (self.index, state_mut.read) {
                (0, 1) | (1, 0) => 2,
                (0, 2) | (2, 0) => 1,
                (1, 2) | (2, 1) => 0,
                _ => unreachable!()
            };
            state_mut.write = next_write;
            state_mut.read = self.index;
        })
    }

    pub fn framebuffer(&mut self) -> &'_ mut Framebuffer {
        self.framebuffer
    }
}

impl Swapchain {
    pub const fn new() -> Self {
        Self {
            buffers: [
                UnsafeCell::new(Framebuffer::new()),
                UnsafeCell::new(Framebuffer::new()),
                UnsafeCell::new(Framebuffer::new()),
            ],
            state: Mutex::new(RefCell::new(SwapchainState {
                read: 0,
                write: 1,
            }))
        }
    }

    pub fn acquire_next<'a>(&'a self) -> SwapchainImage<'a> {
        let (framebuffer, index) = critical_section::with(|cs| {
            let state = self.state.borrow(cs);
            unsafe { (&mut *self.buffers[state.borrow().write as usize].get(), state.borrow().write) }
        });
        SwapchainImage {
            framebuffer,
            index,
            state: &self.state
        }
    }

    pub fn read<'a>(&'a self) -> &'a Framebuffer {
        critical_section::with(|cs| {
            let state = self.state.borrow(cs);
            let read = state.borrow().read;
            unsafe { & *self.buffers[read as usize].get() }
        })
    }
}
