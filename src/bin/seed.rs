use coterie::{
    domain::{
        CreateMemberRequest, MembershipType, MemberStatus, UpdateMemberRequest,
        Event, EventType, EventVisibility,
        Announcement, AnnouncementType,
        Payment, PaymentStatus, PaymentMethod,
    },
    repository::{
        MemberRepository, SqliteMemberRepository,
        EventRepository, SqliteEventRepository,
        AnnouncementRepository, SqliteAnnouncementRepository,
        PaymentRepository, SqlitePaymentRepository,
    },
};
use chrono::{Utc, Duration};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🌱 Starting database seeding...");

    // Initialize database connection
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:coterie.db".to_string());
    
    let db_pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Run migrations first
    println!("📋 Running migrations...");
    sqlx::migrate!("./migrations")
        .run(&db_pool)
        .await?;

    // Initialize repositories
    let member_repo = SqliteMemberRepository::new(db_pool.clone());
    let event_repo = SqliteEventRepository::new(db_pool.clone());
    let announcement_repo = SqliteAnnouncementRepository::new(db_pool.clone());
    let payment_repo = SqlitePaymentRepository::new(db_pool.clone());

    // Seed members
    println!("👥 Creating members...");
    
    // Create admin user
    let admin = member_repo.create(CreateMemberRequest {
        email: "admin@coterie.local".to_string(),
        username: "admin".to_string(),
        full_name: "Admin User".to_string(),
        password: "admin123".to_string(),
        membership_type: MembershipType::Lifetime,
    }).await?;
    
    // Activate admin and add admin marker
    member_repo.update(admin.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        notes: Some("ADMIN".to_string()),
        ..Default::default()
    }).await?;
    
    println!("  ✅ Created admin user (admin@coterie.local / admin123)");

    // Create regular members
    let alice = member_repo.create(CreateMemberRequest {
        email: "alice@example.com".to_string(),
        username: "alice".to_string(),
        full_name: "Alice Johnson".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;
    
    member_repo.update(alice.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        expires_at: Some(Utc::now() + Duration::days(365)),
        ..Default::default()
    }).await?;
    
    let bob = member_repo.create(CreateMemberRequest {
        email: "bob@example.com".to_string(),
        username: "bob".to_string(),
        full_name: "Bob Smith".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Student,
    }).await?;
    
    member_repo.update(bob.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        expires_at: Some(Utc::now() + Duration::days(180)),
        ..Default::default()
    }).await?;
    
    let charlie = member_repo.create(CreateMemberRequest {
        email: "charlie@example.com".to_string(),
        username: "charlie".to_string(),
        full_name: "Charlie Brown".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;
    
    // Charlie's membership is expired
    member_repo.update(charlie.id, UpdateMemberRequest {
        status: Some(MemberStatus::Expired),
        expires_at: Some(Utc::now() - Duration::days(30)),
        ..Default::default()
    }).await?;
    
    let dave = member_repo.create(CreateMemberRequest {
        email: "dave@example.com".to_string(),
        username: "dave".to_string(),
        full_name: "Dave Wilson".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;
    
    // Dave is pending approval
    
    println!("  ✅ Created 4 test members");

    // Seed events
    println!("📅 Creating events...");
    
    let meeting_event = Event {
        id: Uuid::new_v4(),
        title: "Monthly Members Meeting".to_string(),
        description: "Our regular monthly meeting to discuss club business and upcoming activities.".to_string(),
        event_type: EventType::Meeting,
        visibility: EventVisibility::MembersOnly,
        start_time: Utc::now() + Duration::days(7),
        end_time: Some(Utc::now() + Duration::days(7) + Duration::hours(2)),
        location: Some("Main Conference Room".to_string()),
        max_attendees: Some(50),
        rsvp_required: true,
        created_by: admin.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    event_repo.create(meeting_event.clone()).await?;
    
    let workshop_event = Event {
        id: Uuid::new_v4(),
        title: "Intro to Rust Programming".to_string(),
        description: "Learn the basics of Rust programming language in this hands-on workshop.".to_string(),
        event_type: EventType::Workshop,
        visibility: EventVisibility::Public,
        start_time: Utc::now() + Duration::days(14),
        end_time: Some(Utc::now() + Duration::days(14) + Duration::hours(3)),
        location: Some("Training Lab".to_string()),
        max_attendees: Some(20),
        rsvp_required: true,
        created_by: admin.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let workshop = event_repo.create(workshop_event).await?;
    
    let ctf_event = Event {
        id: Uuid::new_v4(),
        title: "Spring CTF Challenge".to_string(),
        description: "Compete in our spring Capture The Flag competition!".to_string(),
        event_type: EventType::CTF,
        visibility: EventVisibility::Public,
        start_time: Utc::now() + Duration::days(21),
        end_time: Some(Utc::now() + Duration::days(22)),
        location: Some("Online".to_string()),
        max_attendees: None,
        rsvp_required: false,
        created_by: admin.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    event_repo.create(ctf_event).await?;
    
    // Register some attendees
    event_repo.register_attendance(workshop.id, alice.id).await?;
    event_repo.register_attendance(workshop.id, bob.id).await?;
    
    println!("  ✅ Created 3 events with attendance");

    // Seed announcements
    println!("📢 Creating announcements...");
    
    let news_announcement = Announcement {
        id: Uuid::new_v4(),
        title: "Welcome to Coterie!".to_string(),
        content: "We're excited to launch our new member management system. This platform will help us better organize our activities and keep everyone connected.".to_string(),
        announcement_type: AnnouncementType::News,
        is_public: true,
        featured: true,
        published_at: Some(Utc::now() - Duration::days(1)),
        created_by: admin.id,
        created_at: Utc::now() - Duration::days(1),
        updated_at: Utc::now() - Duration::days(1),
    };
    announcement_repo.create(news_announcement).await?;
    
    let achievement = Announcement {
        id: Uuid::new_v4(),
        title: "Team Wins Regional CTF Competition".to_string(),
        content: "Congratulations to our CTF team for taking first place in the regional competition! Great work everyone!".to_string(),
        announcement_type: AnnouncementType::Achievement,
        is_public: true,
        featured: false,
        published_at: Some(Utc::now() - Duration::days(5)),
        created_by: admin.id,
        created_at: Utc::now() - Duration::days(5),
        updated_at: Utc::now() - Duration::days(5),
    };
    announcement_repo.create(achievement).await?;
    
    let meeting_announcement = Announcement {
        id: Uuid::new_v4(),
        title: "Next Meeting: Security Best Practices".to_string(),
        content: "Join us for our next meeting where we'll discuss security best practices and recent vulnerabilities.".to_string(),
        announcement_type: AnnouncementType::Meeting,
        is_public: false,
        featured: false,
        published_at: Some(Utc::now()),
        created_by: admin.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    announcement_repo.create(meeting_announcement).await?;
    
    println!("  ✅ Created 3 announcements");

    // Seed payments
    println!("💳 Creating payment records...");
    
    let alice_payment = Payment {
        id: Uuid::new_v4(),
        member_id: alice.id,
        amount_cents: 5000, // $50.00
        currency: "USD".to_string(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Stripe,
        stripe_payment_id: Some("pi_test_alice123".to_string()),
        description: "Annual membership dues".to_string(),
        paid_at: Some(Utc::now() - Duration::days(30)),
        created_at: Utc::now() - Duration::days(30),
        updated_at: Utc::now() - Duration::days(30),
    };
    payment_repo.create(alice_payment).await?;
    
    let bob_payment = Payment {
        id: Uuid::new_v4(),
        member_id: bob.id,
        amount_cents: 2500, // $25.00 (student discount)
        currency: "USD".to_string(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Manual,
        stripe_payment_id: None,
        description: "Student membership dues (6 months)".to_string(),
        paid_at: Some(Utc::now() - Duration::days(15)),
        created_at: Utc::now() - Duration::days(15),
        updated_at: Utc::now() - Duration::days(15),
    };
    payment_repo.create(bob_payment).await?;
    
    let charlie_payment = Payment {
        id: Uuid::new_v4(),
        member_id: charlie.id,
        amount_cents: 5000,
        currency: "USD".to_string(),
        status: PaymentStatus::Failed,
        payment_method: PaymentMethod::Stripe,
        stripe_payment_id: Some("pi_test_charlie_failed".to_string()),
        description: "Membership renewal - payment failed".to_string(),
        paid_at: None,
        created_at: Utc::now() - Duration::days(35),
        updated_at: Utc::now() - Duration::days(35),
    };
    payment_repo.create(charlie_payment).await?;
    
    let dave_pending = Payment {
        id: Uuid::new_v4(),
        member_id: dave.id,
        amount_cents: 5000,
        currency: "USD".to_string(),
        status: PaymentStatus::Pending,
        payment_method: PaymentMethod::Stripe,
        stripe_payment_id: Some("pi_test_dave_pending".to_string()),
        description: "Initial membership dues".to_string(),
        paid_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    payment_repo.create(dave_pending).await?;
    
    println!("  ✅ Created 4 payment records");

    println!("\n✨ Database seeding complete!");
    println!("\n📝 Test credentials:");
    println!("  Admin: admin@coterie.local / admin123");
    println!("  Users: alice@example.com, bob@example.com, charlie@example.com, dave@example.com");
    println!("  Password for all test users: password123");
    
    Ok(())
}