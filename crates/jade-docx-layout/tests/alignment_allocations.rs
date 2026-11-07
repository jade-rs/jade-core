//! Resolving paragraph alignment runs for body and table lines. Token matching
//! must not allocate, including for mixed-case and unknown OOXML values.

use jade_docx_layout::line_wrap::resolve_physical_paragraph_alignment;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count_allocation() {
    if COUNTING.try_with(Cell::get).unwrap_or(false) {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        System.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_allocation();
        System.realloc(ptr, layout, size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn resolving_alignment_does_not_allocate_per_line() {
    let tokens = [
        "left",
        "right",
        "center",
        "start",
        "end",
        "both",
        "justify",
        "distribute",
        "thaiDistribute",
        "lowKashida",
        "mediumKashida",
        "highKashida",
        " \tCeNtEr\n",
        "\u{2003}StArT\u{3000}",
        "unknown",
        "左右",
    ];
    // Initialize the thread-local counter before measuring only the resolver.
    ALLOCATIONS.with(|count| count.set(0));
    COUNTING.with(|enabled| enabled.set(true));
    for _ in 0..1_000 {
        for token in tokens {
            for bidi in [false, true] {
                std::hint::black_box(resolve_physical_paragraph_alignment(
                    std::hint::black_box(token),
                    std::hint::black_box(bidi),
                ));
            }
        }
    }
    COUNTING.with(|enabled| enabled.set(false));
    let allocations = ALLOCATIONS.with(Cell::get);
    eprintln!("alignment resolutions=32000 allocations={allocations}");
    assert_eq!(allocations, 0, "alignment resolution allocated per line");
}
