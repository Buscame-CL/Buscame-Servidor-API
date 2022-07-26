use crate::api;
use crate::oauth2;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sled31 as sled;
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("failed to commit")]
struct CommitError;

#[derive(Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    #[serde(default = "meta_default", skip_serializing_if = "meta_is_null")]
    pub meta: serde_cbor::Value,
    pub token: oauth2::SavedToken,
}

fn meta_default() -> serde_cbor::Value {
    serde_cbor::Value::Null
}

pub(crate) fn meta_is_null(value: &serde_cbor::Value) -> bool {
    *value == serde_cbor::Value::Null
}

#[derive(Serialize, Deserialize)]
pub struct User {
    pub user_id: String,
    pub login: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct PDFItem {
    /// Name of the request.
    pub name: String,
    /// RUT requested.
    #[serde(default)]
    pub rut: Option<String>,
    /// The URL of a pdf.
    pub pdf_url: String,
    /// User who requested the song.
    #[serde(default)]
    pub user: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Requester {
    pub current: Option<RequesterItem>,
    pub items: Vec<RequesterItem>,
    #[serde(default)]
    pub last_update: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RequesterEntry {
    pub user_login: String,
    pub last_update: Option<DateTime<Utc>>,
}

impl Key {
    /// Serialize the current key.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        Ok(serde_cbor::to_vec(self)?)
    }

    /// Deserialize a key.
    pub fn deserialize(bytes: &[u8]) -> Result<Key> {
        Ok(serde_cbor::from_slice(bytes)?)
    }
}

impl<'de> serde::Deserialize<'de> for Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        use serde::de::Error;

        return deserializer.deserialize_seq(KeyVisitor);

        struct KeyVisitor;

        impl<'de> serde::de::Visitor<'de> for KeyVisitor {
            type Value = Key;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid key")
            }

            #[inline]
            fn visit_seq<A>(self, mut visitor: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let ns = visitor.next_element::<String>()?;

                let ns = match ns {
                    Some(ns) => ns,
                    None => return Err(Error::custom("expected namespace element")),
                };

                let key = match ns.as_str() {
                    "requester" => match visitor.next_element::<String>()? {
                        Some(user_login) => Key::Requester { user_login },
                        None => Key::Requesters,
                    },
                    "connections" => {
                        let user_id = visitor
                            .next_element::<String>()?
                            .ok_or_else(|| Error::custom("expected: name"))?;

                        match visitor.next_element::<String>()? {
                            Some(id) => Key::Connection { user_id, id },
                            None => Key::ConnectionsByUserId { user_id },
                        }
                    }
                    "key-to-user-id" => {
                        let key = visitor
                            .next_element::<String>()?
                            .ok_or_else(|| Error::custom("expected: key"))?;

                        Key::KeyToUserId { key }
                    }
                    "user-id-to-key" => {
                        let user_id = visitor
                            .next_element::<String>()?
                            .ok_or_else(|| Error::custom("expected: user_id"))?;

                        Key::UserIdToKey { user_id }
                    }
                    "user" => {
                        let user_id = visitor
                            .next_element::<String>()?
                            .ok_or_else(|| Error::custom("expected: user_id"))?;

                        Key::User { user_id }
                    }
                    "requester-releases" => {
                        let user = visitor
                            .next_element::<String>()?
                            .ok_or_else(|| Error::custom("expected: user"))?;

                        let repo = visitor
                            .next_element::<String>()?
                            .ok_or_else(|| Error::custom("expected: repo"))?;

                        Key::requesterReleases { user, repo }
                    }
                    _ => {
                        let mut args = Vec::new();

                        while let Some(value) = visitor.next_element::<serde_cbor::Value>()? {
                            args.push(value);
                        }

                        Key::Unsupported(ns, args)
                    }
                };

                Ok(key)
            }
        }
    }
}

impl serde::Serialize for Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq as _;

        let mut seq = serializer.serialize_seq(None)?;

        match self {
            Self::Requesters => {
                seq.serialize_element("Requester")?;
            }
            Self::Requester { ref user_login } => {
                seq.serialize_element("Requester")?;
                seq.serialize_element(user_login)?;
            }
            Self::Connection {
                ref user_id,
                ref id,
            } => {
                seq.serialize_element("connections")?;
                seq.serialize_element(user_id)?;
                seq.serialize_element(id)?;
            }
            Self::ConnectionsByUserId { ref user_id } => {
                seq.serialize_element("connections")?;
                seq.serialize_element(user_id)?;
            }
            Self::UserIdToKey { ref user_id } => {
                seq.serialize_element("user-id-to-key")?;
                seq.serialize_element(user_id)?;
            }
            Self::KeyToUserId { ref key } => {
                seq.serialize_element("key-to-user-id")?;
                seq.serialize_element(key)?;
            }
            Self::User { ref user_id } => {
                seq.serialize_element("user")?;
                seq.serialize_element(user_id)?;
            }
            Self::Unsupported(ref ns, ref args) => {
                seq.serialize_element(ns)?;

                for value in args {
                    seq.serialize_element(value)?;
                }
            }
        }

        seq.end()
    }
}

#[derive(Clone)]
pub struct Database {
    tree: sled::Tree,
}

impl Database {
    /// Open a new database instance.
    pub fn load(tree: sled::Tree) -> Result<Database> {
        Ok(Self { tree })
    }

    /// Get information on the given user.
    pub fn list_Requesters(&self) -> Result<Vec<RequesterEntry>> {
        let key = Key::Requesters.serialize()?;
        let prefix = &key[..(key.len() - 1)];

        let mut out = Vec::new();

        for result in self.tree.range(prefix..) {
            let (key, value) = result?;

            match Key::deserialize(key.as_ref())? {
                Key::Requester { ref user_login } => {
                    if let Some(partial) = Self::deserialize::<RequesterPartial>(&value).ok() {
                        out.push(RequesterEntry {
                            user_login: user_login.to_string(),
                            last_update: partial.last_update,
                        });
                    }
                }
                _ => break,
            }
        }

        return Ok(out);

        #[derive(Debug, Deserialize, Serialize)]
        pub struct RequesterPartial {
            pub last_update: Option<DateTime<Utc>>,
        }
    }

    /// Get data for a single Requester.
    pub fn get_Requester(&self, user_login: &str) -> Result<Option<Requester>> {
        let key = Key::Requester {
            user_login: user_login.to_string(),
        };

        self.get::<Requester>(&key)
    }

    /// Get data for a single Requester.
    pub fn insert_Requester(&self, user_login: &str, Requester: Requester) -> Result<()> {
        let key = Key::Requester {
            user_login: user_login.to_string(),
        };

        self.insert(&key, Requester)
    }

    /// Get information on the given user.
    pub fn insert_user(&self, user_id: &str, user: User) -> Result<()> {
        let key = Key::User {
            user_id: user_id.to_string(),
        };

        self.insert(&key, &user)
    }

    /// Get information on the given user.
    pub fn get_user(&self, user_id: &str) -> Result<Option<User>> {
        let key = Key::User {
            user_id: user_id.to_string(),
        };

        self.get::<User>(&key)
    }

    /// Get the current key by the specified user.
    pub fn get_key(&self, user_id: &str) -> Result<Option<String>> {
        let key = Key::UserIdToKey {
            user_id: user_id.to_string(),
        };

        self.get::<String>(&key)
    }

    /// Get the user that corresponds to the given key.
    pub fn get_user_by_key(&self, key: &str) -> Result<Option<User>> {
        let key = Key::KeyToUserId {
            key: key.to_string(),
        };

        let user_id = match self.get::<String>(&key)? {
            Some(user_id) => user_id,
            None => return Ok(None),
        };

        let key = Key::User { user_id };

        self.get::<User>(&key)
    }

    /// Store the given key.
    pub fn insert_key(&self, user_id: &str, key: &str) -> Result<()> {
        let user_to_key = Key::UserIdToKey {
            user_id: user_id.to_string(),
        };

        let key_to_user = Key::KeyToUserId {
            key: key.to_string(),
        };

        let mut tx = self.transaction();
        tx.insert(&user_to_key, &key)?;
        tx.insert(&key_to_user, &user_id)?;
        tx.commit().map_err(|_| CommitError)?;
        Ok(())
    }

    /// Delete the key associated with the specified user.
    pub fn delete_key(&self, user_id: &str) -> Result<()> {
        let user_to_key = Key::UserIdToKey {
            user_id: user_id.to_string(),
        };

        if let Some(key) = self.get::<String>(&user_to_key)? {
            let key_to_user = Key::KeyToUserId { key };

            let mut tx = self.transaction();
            tx.remove(&user_to_key)?;
            tx.remove(&key_to_user)?;
            tx.commit().map_err(|_| CommitError)?;
        }

        Ok(())
    }

    /// Get the connection with the specified ID.
    pub fn get_connection(&self, user_id: &str, id: &str) -> Result<Option<Connection>> {
        let key = Key::Connection {
            user_id: user_id.to_string(),
            id: id.to_string(),
        };

        self.get(&key)
    }

    /// Add the specified connection.
    pub fn add_connection(&self, user_id: &str, connection: &Connection) -> Result<()> {
        let key = Key::Connection {
            user_id: user_id.to_string(),
            id: connection.id.clone(),
        };

        self.insert(&key, connection)
    }

    /// Delete the specified connection.
    pub fn delete_connection(&self, user_id: &str, id: &str) -> Result<()> {
        let key = Key::Connection {
            user_id: user_id.to_string(),
            id: id.to_string(),
        };

        self.remove(&key)
    }

    /// Get all connections for the specified user.
    pub fn connections_by_user(&self, needle_user_id: &str) -> Result<Vec<Connection>> {
        let key = Key::ConnectionsByUserId {
            user_id: needle_user_id.to_string(),
        };

        let key = key.serialize()?;
        let prefix = &key[..(key.len() - 1)];

        let mut out = Vec::new();

        for result in self.tree.range(prefix..) {
            let (key, value) = result?;

            // TODO: do something with the id?
            let _id = match Key::deserialize(key.as_ref())? {
                Key::Connection {
                    ref user_id,
                    ref id,
                } if user_id == needle_user_id => id.to_string(),
                Key::ConnectionsByUserId { ref user_id } if user_id == needle_user_id => {
                    continue;
                }
                _ => break,
            };

            let connection = match serde_cbor::from_slice(value.as_ref()) {
                Ok(connection) => connection,
                Err(e) => {
                    log::warn!("failed to deserialize connection: {}", e);
                    continue;
                }
            };

            out.push(connection);
        }

        Ok(out)
    }

    /// Get all requester releases associated with the specified repository.
    pub fn get_requester_releases(
        &self,
        user: &str,
        repo: &str,
    ) -> Result<Option<Vec<api::pdf::Release>>> {
        let key = Key::requesterReleases {
            user: user.to_string(),
            repo: repo.to_string(),
        };

        self.get(&key)
    }

    /// Write the current requester releases.
    pub fn write_requester_releases(
        &self,
        user: &str,
        repo: &str,
        releases: Vec<api::pdf::Release>,
    ) -> Result<()> {
        let key = Key::requesterReleases {
            user: user.to_string(),
            repo: repo.to_string(),
        };

        self.insert(&key, releases)
    }

    /// Run the given set of operations in a transaction.
    fn transaction(&self) -> Transaction<'_> {
        Transaction {
            tree: &self.tree,
            ops: Vec::new(),
        }
    }

    /// Insert the given key and value.
    fn insert<T>(&self, key: &Key, value: T) -> Result<()>
    where
        T: Serialize,
    {
        let key = key.serialize()?;
        let value = serde_cbor::to_vec(&value)?;
        self.tree.insert(key, value)?;
        Ok(())
    }

    /// Delete the given key.
    fn remove(&self, key: &Key) -> Result<()> {
        let key = key.serialize()?;
        self.tree.remove(key)?;
        Ok(())
    }

    /// Get the value for the given key.
    fn get<T>(&self, key: &Key) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        let key = key.serialize()?;

        let value = match self.tree.get(&key)? {
            Some(value) => value,
            None => return Ok(None),
        };

        let value = match Self::deserialize(&value) {
            Ok(value) => value,
            Err(e) => {
                log::warn!("Ignoring invalid value stored at: {:?}: {}", key, e);
                return Ok(None);
            }
        };

        Ok(Some(value))
    }

    /// Helper function to deserialize a value.
    fn deserialize<T>(value: &sled::IVec) -> serde_cbor::Result<T>
    where
        T: DeserializeOwned,
    {
        serde_cbor::from_slice(&*value)
    }
}

pub enum Operation {
    Remove(Vec<u8>),
    Insert(Vec<u8>, Vec<u8>),
}

struct Transaction<'a> {
    tree: &'a sled::Tree,
    ops: Vec<Operation>,
}

impl Transaction<'_> {
    /// Insert the given key and value.
    pub fn insert<T>(&mut self, key: &Key, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let key = key.serialize()?;
        let value = serde_cbor::to_vec(value)?;
        self.ops.push(Operation::Insert(key, value));
        Ok(())
    }

    /// Delete the given key.
    pub fn remove(&mut self, key: &Key) -> Result<()> {
        let key = key.serialize()?;
        self.ops.push(Operation::Remove(key));
        Ok(())
    }

    /// Commit the current transaction.
    pub fn commit(self) -> sled::TransactionResult<()> {
        let Transaction { tree, ops } = self;

        tree.transaction(move |tree| {
            for op in &ops {
                match op {
                    Operation::Insert(key, value) => {
                        tree.insert(key.clone(), value.clone())?;
                    }
                    Operation::Remove(key) => {
                        tree.remove(key.clone())?;
                    }
                }
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Key;
    use anyhow::Result;

    #[test]
    fn test_subset() -> Result<()> {
        let a = Key::Connection {
            user_id: "100292".to_string(),
            id: "PDF".to_string(),
        };

        let a_bytes = a.serialize()?;

        let b = Key::ConnectionsByUserId {
            user_id: "100292".to_string(),
        };

        let b_bytes = b.serialize()?;

        // everything is a subset *except* the last byte.
        assert!(a_bytes.starts_with(&b_bytes[..(b_bytes.len() - 1)]));

        assert_eq!(a, Key::deserialize(&a_bytes)?);
        assert_eq!(b, Key::deserialize(&b_bytes)?);
        Ok(())
    }
}
