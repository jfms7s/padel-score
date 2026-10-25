#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use padel_score_server::db;
use padel_score_server::players::PlayerChange;
use padel_score_server::store::{Store, StoreError};

async fn store() -> Store {
    Store::new(db::connect("sqlite::memory:").await.unwrap())
}

fn rename(name: &str) -> PlayerChange {
    PlayerChange {
        name: Some(name.to_owned()),
        archived: None,
    }
}

fn archive(archived: bool) -> PlayerChange {
    PlayerChange {
        name: None,
        archived: Some(archived),
    }
}

#[tokio::test]
async fn create_returns_new_then_existing() {
    let store = store().await;
    let (first, created) = store.create_or_get_player("a", "Ana", 10).await.unwrap();
    assert!(created);
    assert_eq!(first.name, "Ana");
    assert!(!first.archived);
    let (second, created) = store.create_or_get_player("a", "Ana", 20).await.unwrap();
    assert!(!created);
    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, 10);
}

#[tokio::test]
async fn create_is_case_and_space_insensitive() {
    let store = store().await;
    let (ana, _) = store.create_or_get_player("a", " ana ", 1).await.unwrap();
    assert_eq!(ana.name, "ana");
    let (again, created) = store.create_or_get_player("a", "ANA", 2).await.unwrap();
    assert!(!created);
    assert_eq!(again.id, ana.id);
    assert_eq!(again.name, "ana");

    let (alvaro, _) = store.create_or_get_player("a", "ÁLVARO", 3).await.unwrap();
    let (again, created) = store.create_or_get_player("a", "álvaro", 4).await.unwrap();
    assert!(!created);
    assert_eq!(again.id, alvaro.id);
    assert_eq!(again.name, "ÁLVARO");
    assert_eq!(store.list_players("a").await.unwrap().len(), 2);
}

#[tokio::test]
async fn create_unarchives_existing() {
    let store = store().await;
    let (ana, _) = store.create_or_get_player("a", "Ana", 1).await.unwrap();
    let archived = store
        .update_player("a", &ana.id, archive(true))
        .await
        .unwrap();
    assert!(archived.archived);
    assert!(store.player("a", &ana.id).await.unwrap().unwrap().archived);

    let (back, created) = store.create_or_get_player("a", "ana", 2).await.unwrap();
    assert!(!created);
    assert_eq!(back.id, ana.id);
    assert!(!back.archived);
    assert!(!store.player("a", &ana.id).await.unwrap().unwrap().archived);
}

#[tokio::test]
async fn blank_or_long_name_is_refused() {
    let store = store().await;
    for bad in ["", "   ", &"é".repeat(41)] {
        let err = store.create_or_get_player("a", bad, 1).await.unwrap_err();
        assert!(matches!(err, StoreError::InvalidPlayerName), "{bad:?}");
    }
    let (ok, created) = store
        .create_or_get_player("a", &"é".repeat(40), 1)
        .await
        .unwrap();
    assert!(created);
    assert_eq!(ok.name.chars().count(), 40);
}

#[tokio::test]
async fn owners_do_not_share_players() {
    let store = store().await;
    let (a, _) = store.create_or_get_player("a", "Ana", 1).await.unwrap();
    let (b, created) = store.create_or_get_player("b", "Ana", 2).await.unwrap();
    assert!(created);
    assert_ne!(a.id, b.id);
    let listed = store.list_players("b").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, b.id);
    assert!(store.player("b", &a.id).await.unwrap().is_none());
}

#[tokio::test]
async fn list_is_ordered_by_name_key_archived_included() {
    let store = store().await;
    for name in ["Rui", "ana", "Marta"] {
        store.create_or_get_player("a", name, 1).await.unwrap();
    }
    let (rui, _) = store.create_or_get_player("a", "rui", 2).await.unwrap();
    store
        .update_player("a", &rui.id, archive(true))
        .await
        .unwrap();
    let names: Vec<_> = store
        .list_players("a")
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["ana", "Marta", "Rui"]);
}

#[tokio::test]
async fn rename_onto_existing_is_conflict() {
    let store = store().await;
    store.create_or_get_player("a", "Ana", 1).await.unwrap();
    let (rui, _) = store.create_or_get_player("a", "Rui", 1).await.unwrap();
    let err = store
        .update_player("a", &rui.id, rename(" ANA "))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict("player_exists")));
    assert_eq!(
        store.player("a", &rui.id).await.unwrap().unwrap().name,
        "Rui"
    );
}

#[tokio::test]
async fn rename_case_only_is_allowed() {
    let store = store().await;
    let (ana, _) = store.create_or_get_player("a", "ana", 1).await.unwrap();
    let renamed = store
        .update_player("a", &ana.id, rename("Ana"))
        .await
        .unwrap();
    assert_eq!(renamed.name, "Ana");
    assert_eq!(renamed.id, ana.id);
    assert_eq!(
        store.player("a", &ana.id).await.unwrap().unwrap().name,
        "Ana"
    );
}

#[tokio::test]
async fn update_other_owners_player_is_not_found() {
    let store = store().await;
    let (ana, _) = store.create_or_get_player("a", "Ana", 1).await.unwrap();
    let err = store
        .update_player("b", &ana.id, rename("Eve"))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    let err = store
        .update_player("a", "no-such-id", archive(true))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
}

#[tokio::test]
async fn update_with_blank_name_is_refused() {
    let store = store().await;
    let (ana, _) = store.create_or_get_player("a", "Ana", 1).await.unwrap();
    let err = store
        .update_player("a", &ana.id, rename("  "))
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidPlayerName));
}
