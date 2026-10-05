use core::pin::{pin, Pin};
use core::sync::atomic::{AtomicUsize, Ordering};

use boomerang_tinymap::{
    key_type, HandleTable, InlineStorage, LinkedHandleTable, SealedTinyMap, TinyMapBuilder,
};

key_type!(NodeKey);

static DROPS: AtomicUsize = AtomicUsize::new(0);

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn handle_types_preserve_shared_reference_thread_traits() {
    assert_send_sync::<HandleTable<NodeKey, Node, 2>>();
    assert_send_sync::<LinkedHandleTable<'static, NodeKey, Node, 2>>();
    assert_send_sync::<boomerang_tinymap::Handle<'static, NodeKey, Node>>();
}

struct Node {
    value: u16,
}

impl Drop for Node {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}

struct SealedImage {
    nodes: SealedTinyMap<NodeKey, Node, InlineStorage<Node, 2>>,
    handles: HandleTable<NodeKey, Node, 2>,
    first: NodeKey,
    second: NodeKey,
}

struct LinkedImage<'a> {
    nodes: LinkedHandleTable<'a, NodeKey, Node, 2>,
}

impl SealedImage {
    fn link(self: Pin<&mut Self>) -> LinkedImage<'_> {
        // SAFETY: `self` is pinned for the returned guard's lifetime. The generated image owns
        // both fields, neither field is moved or structurally mutated after this projection, and
        // the target array covers every pre-sized handle slot exactly once.
        unsafe {
            let image = self.get_unchecked_mut();
            let nodes = image.nodes.as_ref();
            let targets = [
                nodes.get(image.first).unwrap(),
                nodes.get(image.second).unwrap(),
            ];
            let handles = Pin::new_unchecked(&mut image.handles).link(targets);
            LinkedImage { nodes: handles }
        }
    }
}

fn build_image() -> SealedImage {
    let mut nodes = TinyMapBuilder::<NodeKey, Node, InlineStorage<Node, 2>>::inline();
    let first = nodes.try_insert(Node { value: 10 }).unwrap();
    let second = nodes.try_insert(Node { value: 20 }).unwrap();
    SealedImage {
        nodes: nodes.seal(),
        handles: HandleTable::new(),
        first,
        second,
    }
}

#[test]
fn sealed_image_moves_before_pinning_then_links_stable_handles() {
    DROPS.store(0, Ordering::Relaxed);
    {
        let image = build_image();
        let second_key = image.second;
        let moved_image = image;
        let mut pinned_image = pin!(moved_image);

        let linked = pinned_image.as_mut().link();
        let second = linked.nodes.handle(second_key).unwrap();

        assert_eq!(second.get().value, 20);
    }
    assert_eq!(DROPS.load(Ordering::Relaxed), 2);
}

trait Reading {
    fn read(&self) -> u16;
}

struct Sensor(u16);

impl Reading for Sensor {
    fn read(&self) -> u16 {
        self.0
    }
}

struct Actuator(u16);

impl Reading for Actuator {
    fn read(&self) -> u16 {
        self.0
    }
}

struct HeterogeneousImage {
    sensor: Sensor,
    actuator: Actuator,
    handles: HandleTable<NodeKey, dyn Reading, 2>,
}

impl HeterogeneousImage {
    fn link(self: Pin<&mut Self>) -> LinkedHandleTable<'_, NodeKey, dyn Reading, 2> {
        // SAFETY: both concrete targets and the fixed handle table are fields of this pinned root,
        // and the returned guard keeps the root borrowed for every handle's lifetime.
        unsafe {
            let image = self.get_unchecked_mut();
            let targets: [&dyn Reading; 2] = [&image.sensor, &image.actuator];
            Pin::new_unchecked(&mut image.handles).link(targets)
        }
    }
}

#[test]
fn generated_root_owns_concrete_values_and_erases_only_handles() {
    let mut keys = TinyMapBuilder::<NodeKey, (), InlineStorage<(), 2>>::inline();
    let sensor_key = keys.try_insert(()).unwrap();
    let actuator_key = keys.try_insert(()).unwrap();
    let _keys = keys.seal();

    let image = HeterogeneousImage {
        sensor: Sensor(7),
        actuator: Actuator(11),
        handles: HandleTable::new(),
    };
    let mut image = pin!(image);
    let linked = image.as_mut().link();

    assert_eq!(linked.handle(sensor_key).unwrap().get().read(), 7);
    assert_eq!(linked.handle(actuator_key).unwrap().get().read(), 11);
}

#[test]
fn relinking_after_the_previous_guard_drops_preserves_handle_identity() {
    let image = build_image();
    let second_key = image.second;
    let mut image = pin!(image);

    let first_address = {
        let linked = image.as_mut().link();
        linked.nodes.handle(second_key).unwrap().get() as *const Node
    };
    let linked = image.as_mut().link();
    let second_address = linked.nodes.handle(second_key).unwrap().get() as *const Node;

    assert_eq!(first_address, second_address);
}
