//! Implementation of [`PageTableEntry`] and [`PageTable`]

use super::{frame_alloc, FrameTracker, PhysPageNum, StepByOne, VirtAddr, VirtPageNum, PhysAddr};
use alloc::vec;
use alloc::vec::Vec;
use bitflags::*;
use core::fmt::Debug;
// bitflags是比特标志位的crate，它提供了一个宏，可以将u8封装成一个标志位的集合类型，支持一些常见的集合运算。

bitflags! {
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}


/// page table structure
pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

#[derive(Copy, Clone)]  //自动为PageTableEntry实现copy/clone trait
#[repr(C)]
/// page table entry structure
pub struct PageTableEntry {
  pub bits: usize,
}

/// assume that it won't oom when creating/mapping
impl PageTable {
    // 每个应用的地址空间都对应不同的多级列表，即不同的页表的起始地址不一样。
    // 因此pagetable需要保存根节点的物理页号root_ppn作为页表唯一的区分标志.
    // 此外,frames以frametracker的形式保存了页表的所有节点所在的物理页帧
    // 当pagetable生命周期结束后，向量frames里frametracker也会被回收，即
    // 意味着存放多级页表节点的物理帧被回收了.
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        PageTable {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }

    // 多级页表并非创建之后就不再变化,为了mmu能够通过地址转换正确找到应用地址空间
    // 中的数据实际被内核放在内存中位置,os需要动态维护一个虚拟页号到页表项的映射
    // 支持插入/删除键值对
    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(!pte.is_valid(), "vpn {:?} is mapped before mapping", vpn);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }
    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }

    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for i in 0..3 {
            let pte = &mut ppn.get_pte_array()[idxs[i]];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }

    /// Temporarity used to get arguments from user space.
    // 临时创建一个专用手动查页表的pagetable，仅有一个从传入的satp token中得到的
    // 多级页表根节点的物理页号，它的frames字段为空，即不控制任何资源
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }
    // 和create的区别在于不会试图分配物理页帧.一旦在多级页表上遍历遇到空指针就会直接返回none
    pub fn find_pte(&self, vpn: VirtPageNum) -> Option<&PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&PageTableEntry> = None;
        for i in 0..3 {
            let pte = &ppn.get_pte_array()[idxs[i]];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }
    // 调用find_pte来实现,如果能够找到页表项,将页表项copy一份并返回,否则返回一个none
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn)
            .map(|pte| {pte.clone()})
    }
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

impl PageTableEntry {
  // 从一个物理页号和一个页表标志位PTEFlags生成一个页表项实例
  pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {  
    PageTableEntry {
      bits: ppn.0 << 10 | flags.bits as usize,
    }
  }
  // 通过empty方法生成一个全零的页表项，这隐含着该页表项的V标志位为0，因此不是合法的
  pub fn empty() -> Self {
    PageTableEntry {
      bits: 0,
    }
  }
  pub fn ppn(&self) -> PhysPageNum {
    (self.bits >> 10 & ((1usize << 44) - 1)).into()
  }
  pub fn flags(&self) -> PTEFlags {
    PTEFlags::from_bits(self.bits as u8).unwrap()
  }
  // 快速判断一个页表项的V/R/W/X标至位是否为1
  pub fn is_valid(&self) -> bool {
    (self.flags() & PTEFlags::V) != PTEFlags::empty()
  }
  pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
}

/// translate a pointer to a mutable u8 Vec through page table
pub fn translated_byte_buffer(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let ppn = page_table.translate(vpn).unwrap().ppn();
        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.into();
    }
    v
}

// give bare pointer value 
pub fn translated_assign_ptr<T: Debug>(token: usize, ptr: *mut T, value: T) {
    let page_table = PageTable::from_token(token);
    let va = VirtAddr::from(ptr as usize);
    let vpn = va.floor();
    let offset = va.page_offset();
    let ppn = page_table.translate(vpn).unwrap().ppn();
    let pa: PhysAddr = (usize::from(PhysAddr::from(ppn)) + offset).into();
    unsafe {
        let ptr_pa = (pa.0 as *mut T).as_mut().unwrap();
        *ptr_pa = value;
    }
}