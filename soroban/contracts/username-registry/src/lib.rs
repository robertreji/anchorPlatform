#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    UsernameTaken = 1,
    AddressAlreadyOwns = 2,
    NotRegistered = 3,
    NotOwner = 4,
    InvalidUsername = 5,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Username(String), // username -> owner Address
    Owner(Address),   // owner Address -> username (1:1)
}

const LEDGERS_PER_YEAR: u32 = 6_312_000;
const TTL_EXTEND_TO: u32 = LEDGERS_PER_YEAR * 3;
const TTL_THRESHOLD: u32 = LEDGERS_PER_YEAR;

#[contract]
pub struct UsernameRegistry;

fn validate_username(username: &String) -> Result<(), Error> {
    let len = username.len();
    if len < 3 || len > 20 {
        return Err(Error::InvalidUsername);
    }
    let mut buf = [0u8; 20];
    let slice = &mut buf[..len as usize];
    username.copy_into_slice(slice);
    for &byte in slice.iter() {
        let is_lowercase = byte >= b'a' && byte <= b'z';
        let is_digit = byte >= b'0' && byte <= b'9';
        let is_underscore = byte == b'_';
        if !is_lowercase && !is_digit && !is_underscore {
            return Err(Error::InvalidUsername);
        }
    }
    Ok(())
}

#[contractimpl]
impl UsernameRegistry {
    pub fn register(env: Env, owner: Address, username: String) -> Result<(), Error> {
        owner.require_auth();
        validate_username(&username)?;

        let username_key = DataKey::Username(username.clone());
        let owner_key = DataKey::Owner(owner.clone());

        if env.storage().persistent().has(&username_key) {
            return Err(Error::UsernameTaken);
        }
        if env.storage().persistent().has(&owner_key) {
            return Err(Error::AddressAlreadyOwns);
        }

        env.storage().persistent().set(&username_key, &owner);
        env.storage().persistent().set(&owner_key, &username);

        env.storage().persistent().extend_ttl(&username_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        env.storage().persistent().extend_ttl(&owner_key, TTL_THRESHOLD, TTL_EXTEND_TO);

        env.events().publish((symbol_short!("reg"), username), owner);
        Ok(())
    }

    pub fn release(env: Env, owner: Address, username: String) -> Result<(), Error> {
        owner.require_auth();
        let username_key = DataKey::Username(username.clone());
        let owner_key = DataKey::Owner(owner.clone());

        if !env.storage().persistent().has(&username_key) {
            return Err(Error::NotRegistered);
        }

        let current_owner: Address = env.storage().persistent().get(&username_key).unwrap();
        if current_owner != owner {
            return Err(Error::NotOwner);
        }

        env.storage().persistent().remove(&username_key);
        env.storage().persistent().remove(&owner_key);

        env.events().publish((symbol_short!("release"), username), owner);
        Ok(())
    }

    pub fn resolve(env: Env, username: String) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Username(username))
    }

    pub fn reverse(env: Env, owner: Address) -> Option<String> {
        env.storage().persistent().get(&DataKey::Owner(owner))
    }

    pub fn bump(env: Env, username: String) -> Result<(), Error> {
        let username_key = DataKey::Username(username.clone());
        if !env.storage().persistent().has(&username_key) {
            return Err(Error::NotRegistered);
        }
        let owner: Address = env.storage().persistent().get(&username_key).unwrap();
        env.storage().persistent().extend_ttl(&username_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        env.storage().persistent().extend_ttl(&DataKey::Owner(owner), TTL_THRESHOLD, TTL_EXTEND_TO);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events};
    use soroban_sdk::IntoVal;

    #[test]
    fn test_registration_and_resolving() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(UsernameRegistry, ());
        let client = UsernameRegistryClient::new(&env, &contract_id);

        let alice = Address::generate(&env);
        let username = String::from_str(&env, "alice_123");

        // Register alice
        client.register(&alice, &username);

        // Fetch events IMMEDIATELY after registration to catch them before resolve calls clear them
        let events = env.events().all();
        assert_eq!(
            events,
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    (symbol_short!("reg"), username.clone()).into_val(&env),
                    alice.clone().into_val(&env)
                )
            ]
        );

        // Now resolve and reverse
        assert_eq!(client.resolve(&username), Some(alice.clone()));
        assert_eq!(client.reverse(&alice), Some(username.clone()));
    }

    #[test]
    fn test_duplicate_username_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(UsernameRegistry, ());
        let client = UsernameRegistryClient::new(&env, &contract_id);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let username = String::from_str(&env, "alice");

        client.register(&alice, &username);

        let res = client.try_register(&bob, &username);
        assert_eq!(res.err(), Some(Ok(Error::UsernameTaken)));
    }

    #[test]
    fn test_duplicate_owner_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(UsernameRegistry, ());
        let client = UsernameRegistryClient::new(&env, &contract_id);

        let alice = Address::generate(&env);
        let username1 = String::from_str(&env, "alice1");
        let username2 = String::from_str(&env, "alice2");

        client.register(&alice, &username1);

        let res = client.try_register(&alice, &username2);
        assert_eq!(res.err(), Some(Ok(Error::AddressAlreadyOwns)));
    }

    #[test]
    fn test_invalid_usernames_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(UsernameRegistry, ());
        let client = UsernameRegistryClient::new(&env, &contract_id);

        let alice = Address::generate(&env);

        // Too short
        let too_short = String::from_str(&env, "al");
        let res = client.try_register(&alice, &too_short);
        assert_eq!(res.err(), Some(Ok(Error::InvalidUsername)));

        // Too long
        let too_long = String::from_str(&env, "alice_is_longer_than_twenty_characters");
        let res = client.try_register(&alice, &too_long);
        assert_eq!(res.err(), Some(Ok(Error::InvalidUsername)));

        // Uppercase rejected
        let uppercase = String::from_str(&env, "Alice");
        let res = client.try_register(&alice, &uppercase);
        assert_eq!(res.err(), Some(Ok(Error::InvalidUsername)));

        // Special character rejected
        let special = String::from_str(&env, "alice-123");
        let res = client.try_register(&alice, &special);
        assert_eq!(res.err(), Some(Ok(Error::InvalidUsername)));
    }

    #[test]
    fn test_release_only_by_owner() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(UsernameRegistry, ());
        let client = UsernameRegistryClient::new(&env, &contract_id);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let username = String::from_str(&env, "alice");

        client.register(&alice, &username);

        // Bob tries to release Alice's username
        let res = client.try_release(&bob, &username);
        assert_eq!(res.err(), Some(Ok(Error::NotOwner)));

        // Alice releases successfully
        client.release(&alice, &username);
        assert_eq!(client.resolve(&username), None);
        assert_eq!(client.reverse(&alice), None);
    }

    #[test]
    fn test_auth_enforced() {
        let env = Env::default();
        let contract_id = env.register(UsernameRegistry, ());
        let client = UsernameRegistryClient::new(&env, &contract_id);

        let alice = Address::generate(&env);
        let username = String::from_str(&env, "alice");

        let res = client.try_register(&alice, &username);
        assert!(res.is_err());
    }
}
