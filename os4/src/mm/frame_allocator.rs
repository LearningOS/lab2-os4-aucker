//! Implementation of [`FrameAllocator`] which 
//! controls all the frames in the operating system.

use super::{PhysAddr, PhysPageNum};
use crate::config::MEMORY_END;
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};
use lazy_static::*;

/// manage a frame which has the same lifecycle as the tracker
pub struct FrameTracker {
    pub ppn: PhysPageNum,
}

impl FrameTracker {
    pub fn new(ppn: PhysPageNum) -> Self {
        // page cleaning
        let bytes_array = ppn.get_bytes_array();
        for i in bytes_array {
            *i = 0;
        }
        Self { ppn }
    }
}

impl Debug for FrameTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("FrameTracker:PPN={:#x}", self.ppn.0))
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

trait FrameAllocator {
    fn new() -> Self;
    fn alloc(&mut self) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}

pub struct StackFrameAllocator {
    current: usize,
    end: usize,
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    pub fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        self.current = l.0;
        self.end = r.0;
    }
    pub fn remain_num(&self) -> usize {
        self.end - self.current + self.recycled.len()
    }
}

impl FrameAllocator for StackFrameAllocator {
    fn new() -> Self {
        Self {
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }


    // the core: PhysPageFrame allocate and recycle
    fn alloc(&mut self) -> Option<PhysPageNum> {
        // 检查栈recycled是否拥有之前回收的物理页号，有的话直接弹出返回
        if let Some(ppn) = self.recycled.pop() {
            Some(ppn.into())
        } else {
            if self.current == self.end {
                None
            } else {
                // 否则从之前未分配的物理页号区间上分配
                self.current += 1;
                // 在即将返回的时候，使用into将usize转换成物理页号physpagenum
                Some((self.current - 1).into())
            }
        }
    }
    fn dealloc(&mut self, ppn: PhysPageNum) {
        let ppn = ppn.0;
        // validation check
        // 合法性条件：
        // 1.该页面之前分配出去过，物理页号小于current
        // 2.该页面没有处于正在回收状态，物理页号不存在于栈recycled
        if ppn >= self.current || self.recycled
            .iter()
            .find(|&v| {*v == ppn})
            .is_some() {
                panic!("Frame ppn={:#x} has not been allocated!", ppn);
            }
            // recycle
            self.recycled.push(ppn);
    }
}

type FrameAllocatorImpl = StackFrameAllocator;

lazy_static! {
    /// frame allocator instance through lazy_static!
    pub static ref FRAME_ALLOCATOR: UPSafeCell<FrameAllocatorImpl> = 
        unsafe { UPSafeCell::new(FrameAllocatorImpl::new()) }; 
}

/// initiate the frame allocator using "ekernel" and `MEMORY_END`
pub fn init_frame_allocator() {
    extern "C" {
        fn ekernel();
    }
    FRAME_ALLOCATOR.exclusive_access().init(
        PhysAddr::from(ekernel as usize).ceil(), 
        PhysAddr::from(MEMORY_END).floor(),
    );
}

/// allocate a frame
// 返回值不是PhysPageNum，而是包装成了一个FrameTracker
    pub fn frame_alloc() -> Option<FrameTracker> {
        FRAME_ALLOCATOR
            .exclusive_access()
            .alloc()
            .map(FrameTracker::new)
    }
    /// dealloc a frame
    pub fn frame_dealloc(ppn: PhysPageNum) {
        FRAME_ALLOCATOR.exclusive_access().dealloc(ppn);
    }

pub fn frame_remain_num() -> usize {
    FRAME_ALLOCATOR.exclusive_access().remain_num()
}

#[allow(unused)]
/// a simple test for frame allocator
pub fn frame_allocator_test() {
    let mut v: Vec<FrameTracker> = Vec::new();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        info!("{:?}", frame);
        v.push(frame);
    }
    v.clear();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        info!("{:?}", frame);
        v.push(frame);
    }
    drop(v);
    info!("frame_allocator_test passed!");
}