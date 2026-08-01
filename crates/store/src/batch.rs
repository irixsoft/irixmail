use crate::traits_store::WriteOp;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchBuilder {
    ops: Vec<WriteOp>,
}

impl BatchBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ops: Vec::with_capacity(capacity),
        }
    }

    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> &mut Self {
        self.push(WriteOp::Set { key, value })
    }

    pub fn delete(&mut self, key: Vec<u8>) -> &mut Self {
        self.push(WriteOp::Delete { key })
    }

    pub fn add(&mut self, key: Vec<u8>, by: i64) -> &mut Self {
        self.push(WriteOp::Add { key, by })
    }

    pub fn push(&mut self, op: WriteOp) -> &mut Self {
        self.ops.push(op);
        self
    }

    pub fn extend(&mut self, ops: impl IntoIterator<Item = WriteOp>) -> &mut Self {
        self.ops.extend(ops);
        self
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn ops(&self) -> &[WriteOp] {
        &self.ops
    }

    pub fn build(self) -> Vec<WriteOp> {
        self.ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Collection, Key, Subspace};

    fn email_key(account: u32, document: u32) -> Vec<u8> {
        Key::new(Subspace::Property, account, Collection::Email, document).encode()
    }

    fn counter_key(account: u32) -> Vec<u8> {
        Key::new(Subspace::Counter, account, Collection::Email, 0).encode()
    }

    #[test]
    fn new_builder_is_empty() {
        let builder = BatchBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.len(), 0);
        assert_eq!(builder.build(), Vec::new());
    }

    #[test]
    fn methods_collect_a_mixed_write_set() {
        let keep = email_key(1, 1);
        let stale = email_key(1, 2);
        let counter = counter_key(1);

        let mut builder = BatchBuilder::new();
        builder.set(keep.clone(), b"fresh".to_vec());
        builder.delete(stale.clone());
        builder.add(counter.clone(), 3);
        let ops = builder.build();

        assert_eq!(
            ops,
            vec![
                WriteOp::Set {
                    key: keep,
                    value: b"fresh".to_vec(),
                },
                WriteOp::Delete { key: stale },
                WriteOp::Add {
                    key: counter,
                    by: 3
                },
            ]
        );
    }

    #[test]
    fn insertion_order_is_preserved() {
        let mut builder = BatchBuilder::new();
        for document in 1..=4u32 {
            builder.set(email_key(7, document), document.to_be_bytes().to_vec());
        }
        let ops = builder.build();

        let keys: Vec<&[u8]> = ops.iter().map(WriteOp::key).collect();
        assert_eq!(
            keys,
            vec![
                email_key(7, 1).as_slice(),
                email_key(7, 2).as_slice(),
                email_key(7, 3).as_slice(),
                email_key(7, 4).as_slice(),
            ]
        );
    }

    #[test]
    fn builder_can_be_driven_by_reference_in_a_loop() {
        let mut builder = BatchBuilder::new();
        for document in [3u32, 5, 8] {
            builder.delete(email_key(2, document));
        }
        assert_eq!(builder.len(), 3);
        assert_eq!(
            builder.ops()[1],
            WriteOp::Delete {
                key: email_key(2, 5)
            }
        );
    }

    #[test]
    fn push_admits_a_prebuilt_op() {
        let key = email_key(4, 9);
        let mut builder = BatchBuilder::new();
        builder.push(WriteOp::Set {
            key: key.clone(),
            value: b"v".to_vec(),
        });
        assert_eq!(
            builder.ops(),
            &[WriteOp::Set {
                key,
                value: b"v".to_vec()
            }]
        );
    }

    #[test]
    fn extend_appends_an_iterator_in_order() {
        let first = email_key(1, 1);
        let second = email_key(1, 2);
        let mut builder = BatchBuilder::new();
        builder.set(first.clone(), b"a".to_vec());
        builder.extend([
            WriteOp::Delete {
                key: second.clone(),
            },
            WriteOp::Add {
                key: counter_key(1),
                by: -1,
            },
        ]);

        assert_eq!(builder.len(), 3);
        assert_eq!(
            builder.ops()[0],
            WriteOp::Set {
                key: first,
                value: b"a".to_vec()
            }
        );
        assert_eq!(builder.ops()[1], WriteOp::Delete { key: second });
        assert_eq!(
            builder.ops()[2],
            WriteOp::Add {
                key: counter_key(1),
                by: -1
            }
        );
    }

    #[test]
    fn with_capacity_starts_empty() {
        let builder = BatchBuilder::with_capacity(8);
        assert!(builder.is_empty());
        assert_eq!(builder.len(), 0);
    }

    #[test]
    fn ops_borrows_without_consuming() {
        let mut builder = BatchBuilder::new();
        builder.add(counter_key(3), 5);
        assert_eq!(builder.ops().len(), 1);
        builder.add(counter_key(3), 2);
        assert_eq!(builder.len(), 2);
    }
}
