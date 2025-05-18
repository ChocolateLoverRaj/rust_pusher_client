use serde_json::{Map, Number, Value};
use thiserror::Error;

/// Presence user data must contain a key called `user_id` with a number value, and can also have other fields of any types
#[derive(Debug, Clone)]
pub struct PresenceUserData {
    user_id: Number,
    rest_of_map: Map<String, Value>,
}

impl PresenceUserData {
    /// # Panics
    /// If the map also contains a key called `user_id`
    pub fn new(user_id: Number, rest_of_map: Map<String, Value>) -> Self {
        assert!(
            !rest_of_map.contains_key("user_id"),
            "Duplicate key `user_id`"
        );
        if rest_of_map.contains_key("user_info") {
            unimplemented!("The `user_info` field is special and we don't know how to use it")
        }
        Self {
            user_id,
            rest_of_map,
        }
    }

    pub fn user_id(&self) -> &Number {
        &self.user_id
    }

    pub fn rest_of_map(&self) -> &Map<String, Value> {
        &self.rest_of_map
    }

    /// Creates a JSON object that is compatible with Pusher (but doesn't check if it's too big)
    pub fn map_for_pusher(self) -> Map<String, Value> {
        let mut map = self.rest_of_map;
        map.insert("user_id".into(), self.user_id.into());
        map
    }
}

#[derive(Debug, Error)]
pub enum FromMapError {
    #[error("No `user_id` field")]
    NoUserId,
    #[error("`user_id` is not a number")]
    InvalidUserId,
    #[error("Contain the field `user_info` and we don't know what to do with it")]
    ContainsUserInfo,
}

impl TryFrom<Map<String, Value>> for PresenceUserData {
    type Error = FromMapError;

    fn try_from(mut value: Map<String, Value>) -> Result<Self, Self::Error> {
        let user_id = value.remove("user_id").ok_or(FromMapError::NoUserId)?;
        let user_id = user_id.as_number().ok_or(FromMapError::InvalidUserId)?;
        if value.contains_key("user_info") {
            Err(FromMapError::NoUserId)?;
        }
        Ok(Self {
            user_id: user_id.to_owned(),
            rest_of_map: value,
        })
    }
}
