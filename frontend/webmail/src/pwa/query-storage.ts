const DB_NAME = "irixmail-cache";
const STORE = "query";

export interface QueryStorage {
  getItem: (key: string) => Promise<string | null>;
  setItem: (key: string, value: string) => Promise<void>;
  removeItem: (key: string) => Promise<void>;
  clear: () => Promise<void>;
}

function openDb(factory: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = factory.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function transact<T>(
  factory: IDBFactory,
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => IDBRequest,
): Promise<T> {
  const db = await openDb(factory);
  try {
    return await new Promise<T>((resolve, reject) => {
      const request = run(db.transaction(STORE, mode).objectStore(STORE));
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  } finally {
    db.close();
  }
}

export function createQueryStorage(factory: IDBFactory = indexedDB): QueryStorage {
  const attempt = <T>(
    mode: IDBTransactionMode,
    run: (store: IDBObjectStore) => IDBRequest,
    fallback: T,
  ): Promise<T> => {
    try {
      return transact<T>(factory, mode, run).catch(() => fallback);
    } catch {
      return Promise.resolve(fallback);
    }
  };

  return {
    getItem: (key) =>
      attempt<string | null>("readonly", (store) => store.get(key), null).then((value) => value ?? null),
    setItem: (key, value) =>
      attempt<void>("readwrite", (store) => store.put(value, key), undefined).then(() => undefined),
    removeItem: (key) =>
      attempt<void>("readwrite", (store) => store.delete(key), undefined).then(() => undefined),
    clear: () => attempt<void>("readwrite", (store) => store.clear(), undefined).then(() => undefined),
  };
}
