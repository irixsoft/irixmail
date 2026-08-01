import { describe, expect, it } from "vitest";
import { acceptUpdate, reloadOnControllerChange, watchForUpdate } from "./sw-update";

class FakeWorker extends EventTarget {
  state = "installing";
  posted: unknown[] = [];

  postMessage(message: unknown) {
    this.posted.push(message);
  }

  install() {
    this.state = "installed";
    this.dispatchEvent(new Event("statechange"));
  }
}

class FakeRegistration extends EventTarget {
  waiting: FakeWorker | null = null;
  installing: FakeWorker | null = null;

  updateFound(worker: FakeWorker) {
    this.installing = worker;
    this.dispatchEvent(new Event("updatefound"));
  }
}

const registrationOf = (fake: FakeRegistration) => fake as unknown as ServiceWorkerRegistration;

describe("watchForUpdate", () => {
  it("fires immediately when a worker is already waiting on a controlled page", () => {
    const registration = new FakeRegistration();
    registration.waiting = new FakeWorker();
    const seen: unknown[] = [];
    watchForUpdate(registrationOf(registration), () => true, (worker) => seen.push(worker));
    expect(seen).toEqual([registration.waiting]);
  });

  it("stays quiet on first install when nothing controls the page", () => {
    const registration = new FakeRegistration();
    registration.waiting = new FakeWorker();
    const seen: unknown[] = [];
    watchForUpdate(registrationOf(registration), () => false, (worker) => seen.push(worker));

    const fresh = new FakeWorker();
    registration.updateFound(fresh);
    fresh.install();
    expect(seen).toEqual([]);
  });

  it("fires when an update finishes installing while the page is controlled", () => {
    const registration = new FakeRegistration();
    const seen: unknown[] = [];
    watchForUpdate(registrationOf(registration), () => true, (worker) => seen.push(worker));

    const fresh = new FakeWorker();
    registration.updateFound(fresh);
    expect(seen).toEqual([]);
    fresh.install();
    expect(seen).toEqual([fresh]);
  });

  it("stops watching after cleanup", () => {
    const registration = new FakeRegistration();
    const seen: unknown[] = [];
    const stop = watchForUpdate(registrationOf(registration), () => true, (worker) => seen.push(worker));
    stop();

    const fresh = new FakeWorker();
    registration.updateFound(fresh);
    fresh.install();
    expect(seen).toEqual([]);
  });
});

describe("acceptUpdate", () => {
  it("asks the waiting worker to take over", () => {
    const registration = new FakeRegistration();
    registration.waiting = new FakeWorker();
    acceptUpdate(registrationOf(registration));
    expect(registration.waiting.posted).toEqual([{ type: "SKIP_WAITING" }]);
  });

  it("does nothing without a waiting worker", () => {
    expect(() => acceptUpdate(registrationOf(new FakeRegistration()))).not.toThrow();
  });
});

describe("reloadOnControllerChange", () => {
  it("reloads exactly once even if the controller changes again", () => {
    const container = new EventTarget();
    let reloads = 0;
    reloadOnControllerChange(container, () => {
      reloads += 1;
    });
    container.dispatchEvent(new Event("controllerchange"));
    container.dispatchEvent(new Event("controllerchange"));
    expect(reloads).toBe(1);
  });
});
