use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

pub trait Action {
    fn type_(&self) -> &str;
}

#[derive(Clone, Debug)]
pub enum InternalActionType {
    Init,
    Replace,
}

#[derive(Clone, Debug)]
pub struct InternalAction {
    pub kind: InternalActionType,
}
impl Action for InternalAction {
    fn type_(&self) -> &str {
        match self.kind {
            InternalActionType::Init => "@@redux/INIT",
            InternalActionType::Replace => "@@redux/REPLACE",
        }
    }
}

/// 你的业务 action 示例：你可以替换成自己的 enum/struct
#[derive(Clone, Debug)]
pub enum CounterAction {
    Inc,
    Dec,
}
impl Action for CounterAction {
    fn type_(&self) -> &str {
        match self {
            CounterAction::Inc => "counter/inc",
            CounterAction::Dec => "counter/dec",
        }
    }
}

/// AppAction = Internal + Business（像 TS 里 `as A` 的安全替代）
#[derive(Clone, Debug)]
pub enum AppAction<B> {
    Internal(InternalAction),
    Business(B),
}
impl<B: Action> Action for AppAction<B> {
    fn type_(&self) -> &str {
        match self {
            AppAction::Internal(a) => a.type_(),
            AppAction::Business(b) => b.type_(),
        }
    }
}

pub type Reducer<S, A> = dyn Fn(Option<S>, &A) -> S;

#[derive(Clone)]
pub struct Store<S, A: Action + Clone> {
    inner: Rc<StoreInner<S, A>>,
}

pub struct UnsubscribeHandle<S, A: Action + Clone> {
    store: Store<S, A>,
    id: usize,
    active: bool,
}
impl<S, A: Action + Clone> UnsubscribeHandle<S, A> {
    pub fn unsubscribe(&mut self) {
        if !self.active {
            return;
        }
        self.store.inner.unsubscribe(self.id);
        self.active = false;
    }
}

struct StoreInner<S, A: Action> {
    reducer: RefCell<Box<Reducer<S, A>>>,
    state: RefCell<Option<S>>,
    listeners: RefCell<HashMap<usize, Rc<dyn Fn()>>>,
    next_listener_id: RefCell<usize>,
    is_dispatching: RefCell<bool>,
}

impl<S: Clone + 'static, A: Action + Clone + 'static> Store<S, A> {
    pub fn new(reducer: Box<Reducer<S, A>>, preloaded_state: Option<S>, init_action: A) -> Self {
        let store = Self {
            inner: Rc::new(StoreInner {
                reducer: RefCell::new(reducer),
                state: RefCell::new(preloaded_state),
                listeners: RefCell::new(HashMap::new()),
                next_listener_id: RefCell::new(0),
                is_dispatching: RefCell::new(false),
            }),
        };

        // INIT
        store.dispatch(init_action);
        store
    }

    pub fn get_state(&self) -> S {
        self.inner.get_state()
    }

    pub fn dispatch(&self, action: A) -> A {
        self.inner.dispatch(action.clone());
        action
    }

    pub fn subscribe<F>(&self, listener: F) -> UnsubscribeHandle<S, A>
    where
        F: Fn() + 'static,
    {
        self.inner.assert_not_dispatching("store.subscribe()");
        let id = {
            let mut c = self.inner.next_listener_id.borrow_mut();
            let id = *c;
            *c += 1;
            id
        };
        self.inner
            .listeners
            .borrow_mut()
            .insert(id, Rc::new(listener));

        UnsubscribeHandle {
            store: self.clone(),
            id,
            active: true,
        }
    }

    pub fn replace_reducer(&self, next_reducer: Box<Reducer<S, A>>, replace_action: A) {
        self.inner.assert_not_dispatching("store.replace_reducer()");
        *self.inner.reducer.borrow_mut() = next_reducer;
        self.dispatch(replace_action);
    }

    pub fn subscribe_state<F>(&self, mut observer: F) -> UnsubscribeHandle<S, A>
    where
        F: FnMut(S) + 'static,
    {
        observer(self.get_state());
        let store = self.clone();
        self.subscribe(move || observer(store.get_state()))
    }
}

impl<S: Clone, A: Action> StoreInner<S, A> {
    fn assert_not_dispatching(&self, what: &str) {
        if *self.is_dispatching.borrow() {
            panic!(
                "You may not call {} while the reducer is executing.",
                what
            );
        }
    }

    fn get_state(&self) -> S {
        self.assert_not_dispatching("store.get_state()");
        self.state
            .borrow()
            .as_ref()
            .expect("State is not initialized (INIT missing?)")
            .clone()
    }

    fn unsubscribe(&self, id: usize) {
        self.assert_not_dispatching("unsubscribe()");
        self.listeners.borrow_mut().remove(&id);
    }

    fn dispatch(&self, action: A) {
        let t = action.type_();
        if t.is_empty() {
            panic!("Actions may not have an empty \"type\".");
        }
        if *self.is_dispatching.borrow() {
            panic!("Reducers may not dispatch actions.");
        }

        {
            *self.is_dispatching.borrow_mut() = true;
            let prev = self.state.borrow_mut().take();
            let next = (self.reducer.borrow())(prev, &action);
            *self.state.borrow_mut() = Some(next);
            *self.is_dispatching.borrow_mut() = false;
        }

        // snapshot
        let snapshot: Vec<Rc<dyn Fn()>> = {
            let map = self.listeners.borrow();
            map.values().cloned().collect()
        };

        for l in snapshot {
            l();
        }
    }
}

/// ===== 一个最小使用示例（Counter） =====
pub fn example_counter_store() -> Store<i32, AppAction<CounterAction>> {
    let reducer = Box::new(|state: Option<i32>, action: &AppAction<CounterAction>| -> i32 {
        let mut s = state.unwrap_or(0);

        match action {
            AppAction::Internal(_a) => {
                // INIT/REPLACE：通常啥也不做，只保证返回当前/初始 state
                s
            }
            AppAction::Business(b) => {
                match b {
                    CounterAction::Inc => s += 1,
                    CounterAction::Dec => s -= 1,
                }
                s
            }
        }
    });

    let init = AppAction::Internal(InternalAction {
        kind: InternalActionType::Init,
    });

    Store::new(reducer, None, init)
}
