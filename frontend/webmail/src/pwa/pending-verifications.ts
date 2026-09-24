const DB_NAME = "irixmail-push";
const DB_VERSION = 2;
const PENDING = "pending";
const ACCOUNTS = "accounts";

export interface PendingVerification {
  subscriptionId: string;
  code: string;
}

export interface AccountLabel {
  accountId: string;
  label: string;
}

function openDb(factory: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = factory.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(PENDING)) db.createObjectStore(PENDING, { keyPath: "subscriptionId" });
      if (!db.objectStoreNames.contains(ACCOUNTS)) db.createObjectStore(ACCOUNTS, { keyPath: "accountId" });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function transact<T>(
  factory: IDBFactory,
  storeName: string,
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  return openDb(factory).then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const request = run(db.transaction(storeName, mode).objectStore(storeName));
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
  return transact(factory, PENDING, "readwrite", (store) => store.put(entry));
}

export function listPending(factory: IDBFactory = indexedDB): Promise<PendingVerification[]> {
  return transact(factory, PENDING, "readonly", (store) => store.getAll() as IDBRequest<PendingVerification[]>);
}

export function removePending(subscriptionId: string, factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, PENDING, "readwrite", (store) => store.delete(subscriptionId));
}

export function clearPending(factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, PENDING, "readwrite", (store) => store.clear());
}

export function putAccountLabel(entry: AccountLabel, factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, ACCOUNTS, "readwrite", (store) => store.put(entry));
}

export function listAccountLabels(factory: IDBFactory = indexedDB): Promise<AccountLabel[]> {
  return transact(factory, ACCOUNTS, "readonly", (store) => store.getAll() as IDBRequest<AccountLabel[]>);
}

export function removeAccountLabel(accountId: string, factory: IDBFactory = indexedDB): Promise<unknown> {
  return transact(factory, ACCOUNTS, "readwrite", (store) => store.delete(accountId));
}
