use crate::key::Subspace;

const SETTINGS_TAG: u8 = 0x01;

pub fn settings_key() -> Vec<u8> {
    vec![Subspace::Registry.as_byte(), SETTINGS_TAG]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_key_lives_in_the_registry_subspace() {
        let key = settings_key();
        assert_eq!(key[0], Subspace::Registry.as_byte());
        assert_eq!(key.len(), 2);
    }
}
