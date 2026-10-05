#![no_std]

use core::pin::{pin, Pin};

use boomerang_tinymap::{
    key_type, HandleTable, InlineStorage, LinkedHandleTable, SealedTinyMap, TinyMapBuilder,
};

key_type!(NodeKey);

struct Node {
    value: u16,
}

struct SealedImage {
    nodes: SealedTinyMap<NodeKey, Node, InlineStorage<Node, 2>>,
    handles: HandleTable<NodeKey, Node, 2>,
    first: NodeKey,
    second: NodeKey,
}

impl SealedImage {
    fn link(self: Pin<&mut Self>) -> LinkedHandleTable<'_, NodeKey, Node, 2> {
        // SAFETY: the generated root is pinned for the returned guard's lifetime. Its sealed table
        // cannot change shape, and both the targets and fixed handle table are fields of that root.
        unsafe {
            let image = self.get_unchecked_mut();
            let nodes = image.nodes.as_ref();
            let targets = [
                nodes.get(image.first).unwrap(),
                nodes.get(image.second).unwrap(),
            ];
            Pin::new_unchecked(&mut image.handles).link(targets)
        }
    }
}

/// Exercises build, seal, move, pin, link, handle access, and drop without an allocator.
pub fn run() -> u16 {
    let mut nodes = TinyMapBuilder::<NodeKey, Node, InlineStorage<Node, 2>>::inline();
    let first = nodes.try_insert(Node { value: 10 }).unwrap();
    let second = nodes.try_insert(Node { value: 20 }).unwrap();
    let image = SealedImage {
        nodes: nodes.seal(),
        handles: HandleTable::new(),
        first,
        second,
    };

    let moved_image = image;
    let mut pinned_image = pin!(moved_image);
    let linked = pinned_image.as_mut().link();
    linked.handle(second).unwrap().get().value
}
