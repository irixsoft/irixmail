export function watchForUpdate(
  registration: ServiceWorkerRegistration,
  hasController: () => boolean,
  onUpdate: (waiting: ServiceWorker) => void,
): () => void {
  let stopped = false;
  const announce = (worker: ServiceWorker) => {
    if (!stopped && hasController()) onUpdate(worker);
  };

  if (registration.waiting) announce(registration.waiting);

  const cleanups: Array<() => void> = [];
  const onUpdateFound = () => {
    const worker = registration.installing;
    if (!worker) return;
    const onStateChange = () => {
      if (worker.state === "installed") announce(worker);
    };
    worker.addEventListener("statechange", onStateChange);
    cleanups.push(() => worker.removeEventListener("statechange", onStateChange));
  };
  registration.addEventListener("updatefound", onUpdateFound);
  cleanups.push(() => registration.removeEventListener("updatefound", onUpdateFound));

  return () => {
    stopped = true;
    for (const cleanup of cleanups) cleanup();
  };
}

export function acceptUpdate(registration: ServiceWorkerRegistration): void {
  registration.waiting?.postMessage({ type: "SKIP_WAITING" });
}

export function reloadOnControllerChange(container: EventTarget, reload: () => void): () => void {
  let reloaded = false;
  const onChange = () => {
    if (reloaded) return;
    reloaded = true;
    reload();
  };
  container.addEventListener("controllerchange", onChange);
  return () => container.removeEventListener("controllerchange", onChange);
}
