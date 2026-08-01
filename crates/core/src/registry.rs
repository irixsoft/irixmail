use std::future::Future;
use std::pin::Pin;

use parking_lot::Mutex;
use tokio::task::JoinSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Listener,
    Background,
}

impl ServiceKind {
    pub fn label(self) -> &'static str {
        match self {
            ServiceKind::Listener => "listener",
            ServiceKind::Background => "background",
        }
    }
}

pub type ServiceFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

type ServiceFactory = Box<dyn FnOnce() -> ServiceFuture + Send + 'static>;

struct Service {
    name: String,
    kind: ServiceKind,
    factory: ServiceFactory,
}

#[derive(Default)]
pub struct Registry {
    services: Mutex<Vec<Service>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F, Fut>(&self, name: impl Into<String>, kind: ServiceKind, factory: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let factory: ServiceFactory = Box::new(move || Box::pin(factory()));
        self.services.lock().push(Service {
            name: name.into(),
            kind,
            factory,
        });
    }

    pub fn register_listener<F, Fut>(&self, name: impl Into<String>, factory: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.register(name, ServiceKind::Listener, factory);
    }

    pub fn register_background<F, Fut>(&self, name: impl Into<String>, factory: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.register(name, ServiceKind::Background, factory);
    }

    pub fn len(&self) -> usize {
        self.services.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.services.lock().is_empty()
    }

    pub fn registered(&self) -> Vec<(String, ServiceKind)> {
        self.services
            .lock()
            .iter()
            .map(|service| (service.name.clone(), service.kind))
            .collect()
    }

    pub fn start_all(&self) -> JoinSet<()> {
        let pending = std::mem::take(&mut *self.services.lock());
        let mut tasks = JoinSet::new();
        for service in pending {
            tracing::info!(
                service = %service.name,
                kind = service.kind.label(),
                "starting registered service"
            );
            tasks.spawn((service.factory)());
        }
        tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn a_new_registry_is_empty() {
        let registry = Registry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.registered().is_empty());
    }

    #[test]
    fn registering_records_name_kind_and_order() {
        let registry = Registry::new();
        registry.register_listener("imap:993", || async {});
        registry.register_background("queue-manager", || async {});
        registry.register_listener("smtp:25", || async {});

        assert_eq!(registry.len(), 3);
        assert!(!registry.is_empty());
        assert_eq!(
            registry.registered(),
            vec![
                ("imap:993".to_string(), ServiceKind::Listener),
                ("queue-manager".to_string(), ServiceKind::Background),
                ("smtp:25".to_string(), ServiceKind::Listener),
            ]
        );
    }

    #[test]
    fn inspecting_does_not_consume_the_pending_set() {
        let registry = Registry::new();
        registry.register_background("renewal", || async {});

        let _ = registry.registered();
        let _ = registry.len();

        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn the_kind_label_names_each_variant() {
        assert_eq!(ServiceKind::Listener.label(), "listener");
        assert_eq!(ServiceKind::Background.label(), "background");
    }

    #[tokio::test]
    async fn starting_spawns_every_unit_and_runs_its_future() {
        let registry = Registry::new();
        let ran = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let ran = ran.clone();
            registry.register_background("counter", move || async move {
                ran.fetch_add(1, Ordering::SeqCst);
            });
        }

        let mut tasks = registry.start_all();
        while tasks.join_next().await.is_some() {}

        assert_eq!(ran.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn starting_drains_the_registry() {
        let registry = Registry::new();
        registry.register_listener("http:443", || async {});

        let mut first = registry.start_all();
        assert!(registry.is_empty());
        assert!(first.join_next().await.is_some());

        let mut second = registry.start_all();
        assert!(second.join_next().await.is_none());
    }

    #[tokio::test]
    async fn the_factory_runs_at_start_not_at_registration() {
        let registry = Registry::new();
        let built = Arc::new(AtomicUsize::new(0));

        let built_at_registration = built.clone();
        registry.register_background("lazy", move || {
            built_at_registration.fetch_add(1, Ordering::SeqCst);
            async {}
        });

        assert_eq!(built.load(Ordering::SeqCst), 0);

        let mut tasks = registry.start_all();
        while tasks.join_next().await.is_some() {}

        assert_eq!(built.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn registration_through_a_shared_reference_appends() {
        let registry = Arc::new(Registry::new());

        let smtp = registry.clone();
        smtp.register_listener("smtp:25", || async {});

        let queue = registry.clone();
        queue.register_background("queue-manager", || async {});

        assert_eq!(registry.len(), 2);
        let mut tasks = registry.start_all();
        let mut finished = 0;
        while tasks.join_next().await.is_some() {
            finished += 1;
        }
        assert_eq!(finished, 2);
    }
}
