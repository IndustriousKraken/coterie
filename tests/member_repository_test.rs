use coterie::{
    domain::{CreateMemberRequest, MembershipType, MemberStatus},
    repository::{MemberRepository, SqliteMemberRepository},
};
use sqlx::SqlitePool;

#[tokio::test]
async fn test_member_crud() -> anyhow::Result<()> {
    // Create an in-memory SQLite database
    let pool = SqlitePool::connect(":memory:").await?;
    
    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;
    
    // Create repository
    let repo = SqliteMemberRepository::new(pool.clone());
    
    // Test Create
    let create_request = CreateMemberRequest {
        email: "test@example.com".to_string(),
        username: "testuser".to_string(),
        full_name: "Test User".to_string(),
        password: "secure_password123".to_string(),
        membership_type: MembershipType::Regular,
    };
    
    let member = repo.create(create_request).await?;
    assert_eq!(member.email, "test@example.com");
    assert_eq!(member.username, "testuser");
    assert_eq!(member.status, MemberStatus::Pending);
    
    // Test Find by ID
    let found = repo.find_by_id(member.id).await?;
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, member.id);
    
    // Test Find by Email
    let found_by_email = repo.find_by_email("test@example.com").await?;
    assert!(found_by_email.is_some());
    assert_eq!(found_by_email.unwrap().email, "test@example.com");
    
    // Test List
    let members = repo.list(10, 0).await?;
    assert_eq!(members.len(), 1);
    
    // Test Update
    let update = coterie::domain::UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        ..Default::default()
    };
    
    let updated = repo.update(member.id, update).await?;
    assert_eq!(updated.status, MemberStatus::Active);
    
    // Test Delete
    repo.delete(member.id).await?;
    let deleted = repo.find_by_id(member.id).await?;
    assert!(deleted.is_none());
    
    Ok(())
}

#[tokio::test]
async fn test_password_hashing() -> anyhow::Result<()> {
    use coterie::auth;
    
    let password = "my_secure_password";
    let hash = auth::AuthService::hash_password(password).await?;
    
    // Verify the password
    assert!(auth::AuthService::verify_password(password, &hash).await?);
    assert!(!auth::AuthService::verify_password("wrong_password", &hash).await?);
    
    Ok(())
}