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

/// Helper to create a payment record
fn make_payment(
    member_id: Uuid,
    amount_cents: i64,
    status: PaymentStatus,
    method: PaymentMethod,
    description: &str,
    days_ago: i64,
) -> Payment {
    let created = Utc::now() - Duration::days(days_ago);
    let stripe_id = if method == PaymentMethod::Stripe {
        Some(format!("pi_seed_{}", Uuid::new_v4().to_string().chars().take(8).collect::<String>()))
    } else {
        None
    };
    let paid_at = if status == PaymentStatus::Completed { Some(created) } else { None };

    Payment {
        id: Uuid::new_v4(),
        member_id,
        amount_cents,
        currency: "USD".to_string(),
        status,
        payment_method: method,
        stripe_payment_id: stripe_id,
        description: description.to_string(),
        paid_at,
        created_at: created,
        updated_at: created,
    }
}

/// Helper to create an event
fn make_event(
    title: &str,
    description: &str,
    event_type: EventType,
    visibility: EventVisibility,
    days_offset: i64,
    duration_hours: i64,
    location: Option<&str>,
    created_by: Uuid,
) -> Event {
    let start = Utc::now() + Duration::days(days_offset);
    Event {
        id: Uuid::new_v4(),
        title: title.to_string(),
        description: description.to_string(),
        event_type,
        visibility,
        start_time: start,
        end_time: Some(start + Duration::hours(duration_hours)),
        location: location.map(String::from),
        max_attendees: Some(30),
        rsvp_required: true,
        created_by,
        created_at: Utc::now() - Duration::days(days_offset.abs() + 7),
        updated_at: Utc::now() - Duration::days(days_offset.abs() + 7),
    }
}

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

    // Check if database already has data
    let existing_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&db_pool)
        .await?;
    
    if existing_count > 0 {
        println!("⚠️  Database already contains data. Clearing existing data...");
        
        // Clear data in reverse order of foreign key dependencies
        sqlx::query("DELETE FROM payments").execute(&db_pool).await?;
        sqlx::query("DELETE FROM event_attendance").execute(&db_pool).await?;
        sqlx::query("DELETE FROM events").execute(&db_pool).await?;
        sqlx::query("DELETE FROM announcements").execute(&db_pool).await?;
        sqlx::query("DELETE FROM sessions").execute(&db_pool).await?;
        sqlx::query("DELETE FROM member_profiles").execute(&db_pool).await?;
        sqlx::query("DELETE FROM members").execute(&db_pool).await?;
        
        println!("  ✅ Existing data cleared");
    }

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

    // Activate admin and add admin marker - set dues_paid_until via direct SQL
    member_repo.update(admin.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        notes: Some("ADMIN - System administrator".to_string()),
        bypass_dues: Some(true),
        ..Default::default()
    }).await?;

    println!("  ✅ Created admin user (admin@coterie.local / admin123)");

    // Create regular members with realistic histories

    // Alice - Long-term active member, 18 months of payment history
    let alice = member_repo.create(CreateMemberRequest {
        email: "alice@example.com".to_string(),
        username: "alice".to_string(),
        full_name: "Alice Johnson".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;

    // Set Alice's dues paid until 11 months from now (she pays monthly)
    sqlx::query("UPDATE members SET status = 'Active', dues_paid_until = ?, joined_at = ? WHERE id = ?")
        .bind(Utc::now() + Duration::days(30))
        .bind(Utc::now() - Duration::days(540)) // Joined 18 months ago
        .bind(alice.id.to_string())
        .execute(&db_pool)
        .await?;

    // Bob - Student member, 12 months history
    let bob = member_repo.create(CreateMemberRequest {
        email: "bob@example.com".to_string(),
        username: "bob".to_string(),
        full_name: "Bob Smith".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Student,
    }).await?;

    sqlx::query("UPDATE members SET status = 'Active', dues_paid_until = ?, joined_at = ? WHERE id = ?")
        .bind(Utc::now() + Duration::days(45))
        .bind(Utc::now() - Duration::days(365))
        .bind(bob.id.to_string())
        .execute(&db_pool)
        .await?;

    // Charlie - Expired member, was active for 8 months then lapsed
    let charlie = member_repo.create(CreateMemberRequest {
        email: "charlie@example.com".to_string(),
        username: "charlie".to_string(),
        full_name: "Charlie Brown".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;

    sqlx::query("UPDATE members SET status = 'Expired', dues_paid_until = ?, joined_at = ? WHERE id = ?")
        .bind(Utc::now() - Duration::days(45)) // Expired 45 days ago
        .bind(Utc::now() - Duration::days(300))
        .bind(charlie.id.to_string())
        .execute(&db_pool)
        .await?;

    // Dave - Pending new member
    let dave = member_repo.create(CreateMemberRequest {
        email: "dave@example.com".to_string(),
        username: "dave".to_string(),
        full_name: "Dave Wilson".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;
    // Dave stays pending (no updates needed)

    // Eve - Corporate member, 6 months history
    let eve = member_repo.create(CreateMemberRequest {
        email: "eve@techcorp.com".to_string(),
        username: "eve".to_string(),
        full_name: "Eve Martinez".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Corporate,
    }).await?;

    sqlx::query("UPDATE members SET status = 'Active', dues_paid_until = ?, joined_at = ?, notes = ? WHERE id = ?")
        .bind(Utc::now() + Duration::days(60))
        .bind(Utc::now() - Duration::days(180))
        .bind("Corporate sponsor from TechCorp")
        .bind(eve.id.to_string())
        .execute(&db_pool)
        .await?;

    // Frank - Lifetime member (founding member)
    let frank = member_repo.create(CreateMemberRequest {
        email: "frank@example.com".to_string(),
        username: "frank".to_string(),
        full_name: "Frank Chen".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Lifetime,
    }).await?;

    sqlx::query("UPDATE members SET status = 'Active', bypass_dues = 1, joined_at = ?, notes = ? WHERE id = ?")
        .bind(Utc::now() - Duration::days(730)) // Founding member, 2 years ago
        .bind("Founding member - lifetime membership")
        .bind(frank.id.to_string())
        .execute(&db_pool)
        .await?;

    // Grace - Honorary member
    let grace = member_repo.create(CreateMemberRequest {
        email: "grace@university.edu".to_string(),
        username: "grace".to_string(),
        full_name: "Dr. Grace Hopper".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;

    sqlx::query("UPDATE members SET status = 'Honorary', bypass_dues = 1, joined_at = ?, notes = ? WHERE id = ?")
        .bind(Utc::now() - Duration::days(400))
        .bind("Honorary member - Distinguished guest speaker")
        .bind(grace.id.to_string())
        .execute(&db_pool)
        .await?;

    // Henry - Suspended member
    let henry = member_repo.create(CreateMemberRequest {
        email: "henry@example.com".to_string(),
        username: "henry".to_string(),
        full_name: "Henry Adams".to_string(),
        password: "password123".to_string(),
        membership_type: MembershipType::Regular,
    }).await?;

    sqlx::query("UPDATE members SET status = 'Suspended', dues_paid_until = ?, joined_at = ?, notes = ? WHERE id = ?")
        .bind(Utc::now() + Duration::days(90)) // Has paid dues but is suspended
        .bind(Utc::now() - Duration::days(200))
        .bind("Suspended - Code of conduct violation under review")
        .bind(henry.id.to_string())
        .execute(&db_pool)
        .await?;

    println!("  ✅ Created 8 test members with various statuses");

    // Seed events - mix of past and future events
    println!("📅 Creating events...");

    // === PAST EVENTS (for history) ===

    // Past meetings (monthly, going back 6 months)
    for i in 1..=6 {
        let days_ago = i * 30;
        let event = make_event(
            &format!("Monthly Meeting - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
            "Regular monthly meeting to discuss club business and upcoming activities.",
            EventType::Meeting,
            EventVisibility::MembersOnly,
            -days_ago,
            2,
            Some("Main Conference Room"),
            admin.id,
        );
        event_repo.create(event).await?;
    }

    // Past workshops
    let past_workshop1 = make_event(
        "Web Security Fundamentals",
        "Hands-on workshop covering OWASP Top 10 vulnerabilities and how to prevent them.",
        EventType::Workshop,
        EventVisibility::Public,
        -90,
        3,
        Some("Training Lab A"),
        admin.id,
    );
    let pw1 = event_repo.create(past_workshop1).await?;
    event_repo.register_attendance(pw1.id, alice.id).await?;
    event_repo.register_attendance(pw1.id, bob.id).await?;
    event_repo.register_attendance(pw1.id, charlie.id).await?;

    let past_workshop2 = make_event(
        "Python for Security Automation",
        "Learn to automate security tasks with Python scripts and popular libraries.",
        EventType::Workshop,
        EventVisibility::MembersOnly,
        -60,
        4,
        Some("Training Lab B"),
        admin.id,
    );
    let pw2 = event_repo.create(past_workshop2).await?;
    event_repo.register_attendance(pw2.id, alice.id).await?;
    event_repo.register_attendance(pw2.id, eve.id).await?;

    // Past CTF
    let past_ctf = make_event(
        "Winter CTF 2024",
        "Our annual winter Capture The Flag competition. 24 hours of challenges!",
        EventType::CTF,
        EventVisibility::Public,
        -120,
        24,
        Some("Online"),
        admin.id,
    );
    let pctf = event_repo.create(past_ctf).await?;
    event_repo.register_attendance(pctf.id, alice.id).await?;
    event_repo.register_attendance(pctf.id, bob.id).await?;
    event_repo.register_attendance(pctf.id, charlie.id).await?;
    event_repo.register_attendance(pctf.id, frank.id).await?;

    // Past social event
    let past_social = make_event(
        "Holiday Party 2024",
        "End of year celebration with food, drinks, and games!",
        EventType::Social,
        EventVisibility::MembersOnly,
        -45,
        4,
        Some("Community Center"),
        admin.id,
    );
    let ps = event_repo.create(past_social).await?;
    event_repo.register_attendance(ps.id, alice.id).await?;
    event_repo.register_attendance(ps.id, bob.id).await?;
    event_repo.register_attendance(ps.id, eve.id).await?;
    event_repo.register_attendance(ps.id, frank.id).await?;

    // Past training
    let past_training = make_event(
        "New Member Orientation",
        "Welcome session for new members. Learn about our tools, resources, and community guidelines.",
        EventType::Training,
        EventVisibility::MembersOnly,
        -75,
        2,
        Some("Main Conference Room"),
        admin.id,
    );
    event_repo.create(past_training).await?;

    // === UPCOMING EVENTS ===

    // Next monthly meeting
    let next_meeting = make_event(
        "Monthly Meeting - January 2025",
        "First meeting of the new year! We'll review last year's achievements and plan for 2025.",
        EventType::Meeting,
        EventVisibility::MembersOnly,
        7,
        2,
        Some("Main Conference Room"),
        admin.id,
    );
    let nm = event_repo.create(next_meeting).await?;
    event_repo.register_attendance(nm.id, alice.id).await?;
    event_repo.register_attendance(nm.id, bob.id).await?;

    // Upcoming workshop
    let upcoming_workshop = make_event(
        "Intro to Rust Programming",
        "Learn the basics of Rust programming language in this hands-on workshop. Perfect for beginners!",
        EventType::Workshop,
        EventVisibility::Public,
        14,
        3,
        Some("Training Lab A"),
        admin.id,
    );
    let uw = event_repo.create(upcoming_workshop).await?;
    event_repo.register_attendance(uw.id, alice.id).await?;

    // Upcoming CTF
    let upcoming_ctf = make_event(
        "Spring CTF 2025",
        "Compete in our spring Capture The Flag competition! Beginner-friendly with challenges for all skill levels.",
        EventType::CTF,
        EventVisibility::Public,
        28,
        48,
        Some("Online"),
        admin.id,
    );
    event_repo.create(upcoming_ctf).await?;

    // Upcoming social
    let upcoming_social = make_event(
        "Game Night",
        "Board games, video games, and pizza! Bring your favorite games to share.",
        EventType::Social,
        EventVisibility::MembersOnly,
        21,
        4,
        Some("Member Lounge"),
        admin.id,
    );
    event_repo.create(upcoming_social).await?;

    // Upcoming training
    let upcoming_training = make_event(
        "Linux Fundamentals Bootcamp",
        "Two-day intensive training covering Linux system administration basics.",
        EventType::Training,
        EventVisibility::Public,
        35,
        8,
        Some("Training Lab B"),
        admin.id,
    );
    event_repo.create(upcoming_training).await?;

    // Future monthly meetings
    for i in 1..=3 {
        let days_future = 30 + (i * 30);
        let event = make_event(
            &format!("Monthly Meeting - {}", (Utc::now() + Duration::days(days_future)).format("%B %Y")),
            "Regular monthly meeting to discuss club business and upcoming activities.",
            EventType::Meeting,
            EventVisibility::MembersOnly,
            days_future,
            2,
            Some("Main Conference Room"),
            admin.id,
        );
        event_repo.create(event).await?;
    }

    println!("  ✅ Created 20+ events (past and upcoming) with attendance records");

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

    // Seed payments - realistic monthly payment history
    println!("💳 Creating payment records...");
    let mut payment_count = 0;

    // Alice - 18 months of monthly $50 payments (long-term active member)
    // Mix of Stripe and occasional manual payments
    for month in 0..18 {
        let days_ago = month * 30 + 5; // Payments on roughly the 5th of each month
        let method = if month % 4 == 0 { PaymentMethod::Manual } else { PaymentMethod::Stripe };
        let description = if month == 0 {
            "Monthly dues - January 2025".to_string()
        } else {
            format!("Monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y"))
        };

        let payment = make_payment(
            alice.id,
            5000, // $50
            PaymentStatus::Completed,
            method,
            &description,
            days_ago,
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    // Bob - 12 months of student payments ($25/month)
    for month in 0..12 {
        let days_ago = month * 30 + 10;
        let payment = make_payment(
            bob.id,
            2500, // $25 student rate
            PaymentStatus::Completed,
            PaymentMethod::Stripe,
            &format!("Student monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
            days_ago,
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    // Charlie - 8 months of payments, then stopped (expired member)
    for month in 2..10 { // Starting 2 months before expiry, going back 10 months
        let days_ago = 45 + (month - 2) * 30; // 45 days expired + history
        let payment = make_payment(
            charlie.id,
            5000,
            PaymentStatus::Completed,
            PaymentMethod::Stripe,
            &format!("Monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
            days_ago,
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    // Charlie's failed renewal attempt
    let charlie_failed = make_payment(
        charlie.id,
        5000,
        PaymentStatus::Failed,
        PaymentMethod::Stripe,
        "Monthly dues renewal - Payment declined",
        40,
    );
    payment_repo.create(charlie_failed).await?;
    payment_count += 1;

    // Dave - pending payment (new member)
    let dave_pending = make_payment(
        dave.id,
        5000,
        PaymentStatus::Pending,
        PaymentMethod::Stripe,
        "Initial membership dues",
        0,
    );
    payment_repo.create(dave_pending).await?;
    payment_count += 1;

    // Eve - 6 months of corporate payments ($100/month)
    for month in 0..6 {
        let days_ago = month * 30 + 1;
        let payment = make_payment(
            eve.id,
            10000, // $100 corporate rate
            PaymentStatus::Completed,
            PaymentMethod::Stripe,
            &format!("Corporate monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
            days_ago,
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    // Frank - Lifetime membership payment (one-time, 2 years ago)
    let frank_lifetime = make_payment(
        frank.id,
        50000, // $500 lifetime
        PaymentStatus::Completed,
        PaymentMethod::Manual,
        "Lifetime membership - Founding member",
        730,
    );
    payment_repo.create(frank_lifetime).await?;
    payment_count += 1;

    // Henry - 6 months of payments before suspension
    for month in 0..6 {
        let days_ago = month * 30 + 15;
        let payment = make_payment(
            henry.id,
            5000,
            PaymentStatus::Completed,
            PaymentMethod::Stripe,
            &format!("Monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
            days_ago,
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    // Add some variety: a refunded payment
    let refunded_payment = make_payment(
        alice.id,
        5000,
        PaymentStatus::Refunded,
        PaymentMethod::Stripe,
        "Monthly dues - Refunded (duplicate charge)",
        120,
    );
    payment_repo.create(refunded_payment).await?;
    payment_count += 1;

    // Add a workshop fee payment
    let workshop_payment = make_payment(
        bob.id,
        2000, // $20 workshop fee
        PaymentStatus::Completed,
        PaymentMethod::Stripe,
        "Workshop fee - Web Security Fundamentals",
        90,
    );
    payment_repo.create(workshop_payment).await?;
    payment_count += 1;

    println!("  ✅ Created {} payment records with realistic history", payment_count);

    println!("\n✨ Database seeding complete!");
    println!("\n📝 Test credentials (password for all: password123):");
    println!("  Admin:     admin@coterie.local / admin123");
    println!("");
    println!("  Active members:");
    println!("    alice@example.com    - Regular, 18 months history");
    println!("    bob@example.com      - Student, 12 months history");
    println!("    eve@techcorp.com     - Corporate, 6 months history");
    println!("    frank@example.com    - Lifetime founding member");
    println!("");
    println!("  Other statuses:");
    println!("    charlie@example.com  - Expired (lapsed 45 days ago)");
    println!("    dave@example.com     - Pending (new signup)");
    println!("    grace@university.edu - Honorary member");
    println!("    henry@example.com    - Suspended");

    Ok(())
}