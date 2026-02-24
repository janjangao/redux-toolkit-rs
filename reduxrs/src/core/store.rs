use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

pub type ListenerId = u64;

pub type Reducer<S, A> = dyn Fn(&S, &A) -> S + 'static;

pub type Listener<S, A> = dyn FnMut(&S, &A) + 'static;

#[derive(Clone)]
pub struct Store<S, A> {
    inner: Rc<RefCell<Inner<S, A>>>,
}

struct Inner<S, A> {
    reducer: Box<Reducer<S, A>>,
    state: S,

    listeners: BTreeMap<ListenerId, Rc<RefCell<Box<Listener<S, A>>>>>,
    next_listener_id: ListenerId,

    // 防止 reducer 内部重入 dispatch（等价 Redux 的 isDispatching 约束）
    is_reducing: bool,
}

/// 订阅句柄：Drop 自动退订（你也可以手动 unsubscribe）
pub struct Subscription {
    store: Weak<RefCell<dyn AnyUnsubscribe>>,
    id: ListenerId,
    active: bool,
}

// 用 trait 做一次“类型擦除”，让 Subscription 不携带 S/A 泛型
trait AnyUnsubscribe {
    fn unsubscribe_by_id(&mut self, id: ListenerId);
}
impl<S, A> AnyUnsubscribe for Inner<S, A> {
    fn unsubscribe_by_id(&mut self, id: ListenerId) {
        self.listeners.remove(&id);
    }
}

impl Subscription {
    pub fn unsubscribe(mut self) {
        self.drop_impl();
        self.active = false;
    }

    fn drop_impl(&mut self) {
        if !self.active {
            return;
        }
        if let Some(rc) = self.store.upgrade() {
            // 这里用动态分发把 unsubscribe 调回具体 Inner
            let mut borrow = rc.borrow_mut();
            borrow.unsubscribe_by_id(self.id);
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.drop_impl();
    }
}

impl<S, A> Store<S, A> {
    /// createStore / Store::new：核心构造函数
    pub fn new(reducer: impl Fn(&S, &A) -> S + 'static, preloaded_state: S) -> Self {
        let inner = Inner {
            reducer: Box::new(reducer),
            state: preloaded_state,
            listeners: BTreeMap::new(),
            next_listener_id: 0,
            is_reducing: false,
        };
        Self {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    /// Rust 风格：返回一个 state 的克隆快照
    ///（也可以提供 get_state_ref，但会让外部持有 borrow 更容易卡住 dispatch）
    pub fn get_state(&self) -> S
    where
        S: Clone,
    {
        self.inner.borrow().state.clone()
    }

    /// 更接近 Redux：把 action 交给 reducer，更新 state，然后通知订阅者
    pub fn dispatch(&self, action: A) {
        // 1) reducer 计算 next_state（只在这个阶段锁住 inner）
        let (next_state, listeners_snapshot) = {
            let mut inner = self.inner.borrow_mut();

            if inner.is_reducing {
                panic!("Reducers may not dispatch actions (re-entrant dispatch detected).");
            }

            inner.is_reducing = true;
            let next_state = (inner.reducer)(&inner.state, &action);
            inner.state = next_state;
            inner.is_reducing = false;

            // snapshot listeners（确保本轮 dispatch 稳定）
            let snapshot: Vec<_> = inner.listeners.values().cloned().collect();
            (inner.state_ref_clone_for_notify(), snapshot)
        };

        // 2) 通知 listeners（此时不持有 inner 的 borrow）
        for cb in listeners_snapshot {
            cb.borrow_mut()(&next_state, &action);
        }
    }

    /// 订阅：listener 接收 (&state, &action)
    /// 返回 Subscription：drop 自动退订
    pub fn subscribe(&self, listener: impl FnMut(&S, &A) + 'static) -> Subscription {
        let (id, weak_any): (ListenerId, Weak<RefCell<dyn AnyUnsubscribe>>) = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.next_listener_id;
            inner.next_listener_id += 1;

            inner.listeners.insert(
                id,
                Rc::new(RefCell::new(Box::new(listener) as Box<Listener<S, A>>)),
            );

            // 这里做一次类型擦除，让 Subscription 不带泛型
            let erased: Rc<RefCell<dyn AnyUnsubscribe>> = self.inner.clone();
            (id, Rc::downgrade(&erased))
        };

        Subscription {
            store: weak_any,
            id,
            active: true,
        }
    }

    /// 可选：替换 reducer（类似 replaceReducer）
    pub fn replace_reducer(&self, next: impl Fn(&S, &A) -> S + 'static) {
        let mut inner = self.inner.borrow_mut();
        inner.reducer = Box::new(next);
    }
}

impl<S, A> Inner<S, A> {
    // 帮 dispatch 把 state 借用转成可在 borrow 结束后使用的值
    fn state_ref_clone_for_notify(&self) -> S
    where
        S: Clone,
    {
        self.state.clone()
    }
}
