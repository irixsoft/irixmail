// Mirrored by the inline IndexedDB code in public/sw.js — keep names and shapes in sync.
const DB_NAME = "irixmail-push";
const STORE = "pending";

export interface PendingVerification {
  subscriptionId: string;
  code: string;
}

function openDb(factory: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = factory.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE, { keyPath: "subscriptionId" });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function transact<T>(
  factory: IDBFactory,
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  return openDb(factory).then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const request = run(db.transaction(STORE, mode).objectStore(STORE));
        request.onsuccess = () => {
          db.close();
          resolve(request.result);
        };
        request.onerror = () => {
          db.close();
          reject(request.error);
        };
      }),
  );
}

export function putPending(entry: PendingVerification, factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, "readwrite", (store) => store.put(entry));
}

export function listPending(factory: IDBFactory = indexedDB): Promise<PendingVerification[]> {
  return transact(factory, "readonly", (store) => store.getAll() as IDBRequest<PendingVerification[]>);
}

export function removePending(subscriptionId: string, factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, "readwrite", (store) => store.delete(subscriptionId));
}

export function clearPending(factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, "readwrite", (store) => store.clear());
}
