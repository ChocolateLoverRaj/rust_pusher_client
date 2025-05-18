use serde_json::{Map, Number, Value};

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
        if rest_of_map.contains_key("user_id") {
            panic!("Duplicate key `user_id`")
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
