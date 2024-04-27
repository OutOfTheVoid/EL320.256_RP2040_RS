use critical_section::Mutex;
use core::cell::{RefCell, UnsafeCell};
use cortex_m::asm::wfi;

use crate::framebuffer::Framebuffer;

struct SwapchainState {
    read: u32,
    free: Option<u32>,
    pending: Option<u32>,
    write: Option<u32>,
}

pub struct Swapchain {
    buffers: [UnsafeCell<Framebuffer>; 3],
    state: Mutex<RefCell<SwapchainState>>,
}

unsafe impl Send for Swapchain {}
unsafe impl Sync for Swapchain {}

pub struct SwapchainImage<'a> {
    framebuffer: &'a mut Framebuffer,
    state: &'a Mutex<RefCell<SwapchainState>>,
}

impl SwapchainImage<'_> {
    pub fn submit(self) {
        critical_section::with(|cs| {
            let state = self.state.borrow(cs);
            let mut state_mut = state.borrow_mut();
            if let Some(write) = state_mut.write.take() {
                state_mut.pending = Some(write);
            }
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
                write: Some(0),
                free: Some(1),
                pending: None,
                read: 2,
            }))
        }
    }

    pub fn acquire_next<'a>(&'a self) -> SwapchainImage<'a> {
        critical_section::with(|cs| {
            let state = self.state.borrow(cs);
            let mut state_mut = state.borrow_mut();
            let framebuffer = loop {
                let index = if let Some(write) = state_mut.write {
                    Some(write)
                } else if let Some(free) = state_mut.free.take() {
                    state_mut.write = Some(free);
                    Some(free)
                } else {
                    None
                };
                if let Some(index) = index {
                    break unsafe { &mut *self.buffers[index as usize].get() };
                }
                wfi();
            };
            SwapchainImage {
                framebuffer,
                state: &self.state
            }
        })
    }

    pub fn read<'a>(&'a self) -> &'a Framebuffer {
        critical_section::with(|cs| {
            let state = self.state.borrow(cs);
            let mut state_mut = state.borrow_mut();
            if let Some(pending) = state_mut.pending.take() {
                state_mut.free = Some(state_mut.read);
                state_mut.read = pending;
            }
            unsafe { &*self.buffers[state_mut.read as usize].get() }
        })
    }
}
