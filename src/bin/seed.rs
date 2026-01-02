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
use fake::Fake;
use fake::faker::name::en::{FirstName, LastName};
use rand::Rng;
use rand::seq::SliceRandom;

// Configuration
const MEMBER_COUNT: usize = 100;
const MONTHS_OF_HISTORY: i64 = 24; // 2 years of events/payments

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

/// Generate a unique username from a name
fn make_username(first: &str, last: &str, rng: &mut impl Rng) -> String {
    let styles = [
        format!("{}{}", first.to_lowercase(), last.to_lowercase().chars().next().unwrap_or('x')),
        format!("{}.{}", first.to_lowercase(), last.to_lowercase()),
        format!("{}_{}", first.to_lowercase(), rng.gen_range(10..99)),
        format!("{}{}", first.to_lowercase().chars().next().unwrap_or('x'), last.to_lowercase()),
    ];
    styles.choose(rng).unwrap().clone()
}

/// Member generation configuration
struct MemberConfig {
    status: MemberStatus,
    membership_type: MembershipType,
    months_active: i64,
    bypass_dues: bool,
    notes: Option<String>,
}

fn generate_member_config(rng: &mut impl Rng) -> MemberConfig {
    // Distribution of member statuses (realistic for a club)
    // 70% active, 10% expired, 8% pending, 5% suspended, 5% honorary, 2% lifetime
    let roll: u8 = rng.gen_range(0..100);

    let (status, membership_type, months_active, bypass_dues, notes) = match roll {
        0..=69 => {
            // Active members
            let mem_type = match rng.gen_range(0..100) {
                0..=70 => MembershipType::Regular,
                71..=90 => MembershipType::Student,
                91..=98 => MembershipType::Corporate,
                _ => MembershipType::Lifetime,
            };
            let months = rng.gen_range(1..=24);
            (MemberStatus::Active, mem_type, months, false, None)
        }
        70..=79 => {
            // Expired members (were active for a while, then lapsed)
            let months = rng.gen_range(3..=12);
            (MemberStatus::Expired, MembershipType::Regular, months, false, None)
        }
        80..=87 => {
            // Pending members (new signups)
            (MemberStatus::Pending, MembershipType::Regular, 0, false, None)
        }
        88..=92 => {
            // Suspended members
            let months = rng.gen_range(2..=8);
            (MemberStatus::Suspended, MembershipType::Regular, months, false,
             Some("Suspended - under review".to_string()))
        }
        93..=97 => {
            // Honorary members
            (MemberStatus::Honorary, MembershipType::Regular, 0, true,
             Some("Honorary member".to_string()))
        }
        _ => {
            // Lifetime members
            let months = rng.gen_range(12..=36);
            (MemberStatus::Active, MembershipType::Lifetime, months, true,
             Some("Lifetime member".to_string()))
        }
    };

    MemberConfig {
        status,
        membership_type,
        months_active,
        bypass_dues,
        notes,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    println!("🌱 Starting database seeding...");
    println!("   Generating {} members with {} months of history", MEMBER_COUNT, MONTHS_OF_HISTORY);

    let mut rng = rand::thread_rng();

    // Initialize database connection
    let database_url = std::env::var("COTERIE_DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:coterie.db".to_string());

    let db_pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Run migrations first
    println!("  Running migrations...");
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
        sqlx::query("DELETE FROM csrf_tokens").execute(&db_pool).await?;
        sqlx::query("DELETE FROM member_profiles").execute(&db_pool).await?;
        sqlx::query("DELETE FROM members").execute(&db_pool).await?;

        println!("    Existing data cleared");
    }

    // Initialize repositories
    let member_repo = SqliteMemberRepository::new(db_pool.clone());
    let event_repo = SqliteEventRepository::new(db_pool.clone());
    let announcement_repo = SqliteAnnouncementRepository::new(db_pool.clone());
    let payment_repo = SqlitePaymentRepository::new(db_pool.clone());

    // =========================================================================
    // MEMBERS
    // =========================================================================
    println!("👥 Creating members...");

    // Track created members for later use
    let mut all_members: Vec<(Uuid, MemberConfig)> = Vec::new();
    let mut used_usernames: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut used_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Create admin user first
    let admin = member_repo.create(CreateMemberRequest {
        email: "admin@coterie.local".to_string(),
        username: "admin".to_string(),
        full_name: "Admin User".to_string(),
        password: "admin123".to_string(),
        membership_type: MembershipType::Lifetime,
    }).await?;

    member_repo.update(admin.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        notes: Some("ADMIN - System administrator".to_string()),
        bypass_dues: Some(true),
        ..Default::default()
    }).await?;

    used_usernames.insert("admin".to_string());
    used_emails.insert("admin@coterie.local".to_string());
    println!("    Created admin user (admin@coterie.local / admin123)");

    // Create the known test users for predictable testing
    let test_users: [(&str, &str, &str, MembershipType, MemberStatus, i64, bool, Option<&str>); 4] = [
        ("alice@example.com", "alice", "Alice Johnson", MembershipType::Regular, MemberStatus::Active, 18, false, None),
        ("bob@example.com", "bob", "Bob Smith", MembershipType::Student, MemberStatus::Active, 12, false, None),
        ("charlie@example.com", "charlie", "Charlie Brown", MembershipType::Regular, MemberStatus::Expired, 8, false, None),
        ("dave@example.com", "dave", "Dave Wilson", MembershipType::Regular, MemberStatus::Pending, 0, false, None),
    ];

    for (email, username, full_name, mem_type, status, months, bypass, notes) in test_users {
        let member = member_repo.create(CreateMemberRequest {
            email: email.to_string(),
            username: username.to_string(),
            full_name: full_name.to_string(),
            password: "password123".to_string(),
            membership_type: mem_type.clone(),
        }).await?;

        let dues_until = if status == MemberStatus::Active {
            Some(Utc::now() + Duration::days(30))
        } else if status == MemberStatus::Expired {
            Some(Utc::now() - Duration::days(45))
        } else {
            None
        };

        let joined = Utc::now() - Duration::days(months * 30);

        sqlx::query("UPDATE members SET status = ?, dues_paid_until = ?, joined_at = ?, bypass_dues = ?, notes = ? WHERE id = ?")
            .bind(format!("{:?}", status))
            .bind(dues_until)
            .bind(joined)
            .bind(bypass)
            .bind(notes)
            .bind(member.id.to_string())
            .execute(&db_pool)
            .await?;

        all_members.push((member.id, MemberConfig {
            status: status.clone(),
            membership_type: mem_type,
            months_active: months,
            bypass_dues: bypass,
            notes: None,
        }));
        used_usernames.insert(username.to_string());
        used_emails.insert(email.to_string());
    }
    println!("    Created 4 known test users");

    // Generate random members
    let random_count = MEMBER_COUNT.saturating_sub(5); // 5 = admin + 4 test users
    let mut generated = 0;
    let mut attempts = 0;
    const MAX_ATTEMPTS: usize = 1000;

    while generated < random_count && attempts < MAX_ATTEMPTS {
        attempts += 1;

        let first_name: String = FirstName().fake_with_rng(&mut rng);
        let last_name: String = LastName().fake_with_rng(&mut rng);
        let full_name = format!("{} {}", first_name, last_name);

        // Generate unique username
        let mut username = make_username(&first_name, &last_name, &mut rng);
        if used_usernames.contains(&username) {
            username = format!("{}_{}", username, rng.gen_range(100..999));
        }
        if used_usernames.contains(&username) {
            continue; // Try again
        }

        // Generate unique email
        let email_domains = ["example.com", "test.local", "mail.test", "demo.org", "sample.net"];
        let domain = email_domains.choose(&mut rng).unwrap();
        let mut email = format!("{}@{}", username, domain);
        if used_emails.contains(&email) {
            email = format!("{}.{}@{}", first_name.to_lowercase(), last_name.to_lowercase(), domain);
        }
        if used_emails.contains(&email) {
            continue; // Try again
        }

        let config = generate_member_config(&mut rng);

        let member = member_repo.create(CreateMemberRequest {
            email: email.clone(),
            username: username.clone(),
            full_name,
            password: "password123".to_string(),
            membership_type: config.membership_type.clone(),
        }).await?;

        // Calculate dates based on config
        let months_ago = config.months_active;
        let joined = Utc::now() - Duration::days(months_ago * 30 + rng.gen_range(0..30));

        let dues_until = match config.status {
            MemberStatus::Active => {
                if config.bypass_dues {
                    None
                } else {
                    Some(Utc::now() + Duration::days(rng.gen_range(7..90)))
                }
            }
            MemberStatus::Expired => {
                Some(Utc::now() - Duration::days(rng.gen_range(1..90)))
            }
            _ => None,
        };

        sqlx::query("UPDATE members SET status = ?, dues_paid_until = ?, joined_at = ?, bypass_dues = ?, notes = ? WHERE id = ?")
            .bind(format!("{:?}", config.status))
            .bind(dues_until)
            .bind(joined)
            .bind(config.bypass_dues)
            .bind(&config.notes)
            .bind(member.id.to_string())
            .execute(&db_pool)
            .await?;

        all_members.push((member.id, config));
        used_usernames.insert(username);
        used_emails.insert(email);
        generated += 1;
    }

    println!("    Generated {} random members", generated);

    // Count by status
    let mut status_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, config) in &all_members {
        *status_counts.entry(format!("{:?}", config.status)).or_insert(0) += 1;
    }
    println!("    Status distribution: {:?}", status_counts);

    // =========================================================================
    // EVENTS
    // =========================================================================
    println!("📅 Creating events...");
    let mut event_count = 0;

    let event_templates = [
        ("Monthly Meeting", EventType::Meeting, EventVisibility::MembersOnly, 2, "Main Conference Room"),
        ("Security Workshop", EventType::Workshop, EventVisibility::Public, 3, "Training Lab A"),
        ("CTF Competition", EventType::CTF, EventVisibility::Public, 8, "Online"),
        ("Social Mixer", EventType::Social, EventVisibility::MembersOnly, 3, "Member Lounge"),
        ("Training Session", EventType::Training, EventVisibility::MembersOnly, 4, "Training Lab B"),
    ];

    let workshop_topics = [
        "Web Security Fundamentals", "Python for Automation", "Network Forensics",
        "Malware Analysis Basics", "Cloud Security", "Intro to Rust",
        "Reverse Engineering 101", "Incident Response", "OSINT Techniques",
        "Container Security", "API Security Testing", "Threat Modeling",
    ];

    let social_events = [
        "Game Night", "Holiday Party", "Summer BBQ", "Movie Night",
        "Hackathon Kickoff", "New Member Welcome", "Anniversary Celebration",
    ];

    // Generate past events (last 12 months)
    for month in 1..=12 {
        let days_ago = month * 30;

        // Monthly meeting
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
        let created_event = event_repo.create(event).await?;
        event_count += 1;

        // Add random attendees
        let attendee_count = rng.gen_range(5..20);
        let mut shuffled: Vec<_> = all_members.iter().collect();
        shuffled.shuffle(&mut rng);
        for (member_id, _) in shuffled.iter().take(attendee_count) {
            let _ = event_repo.register_attendance(created_event.id, *member_id).await;
        }

        // Occasional workshop
        if month % 2 == 0 {
            let topic = workshop_topics.choose(&mut rng).unwrap();
            let event = make_event(
                topic,
                &format!("Hands-on workshop: {}", topic),
                EventType::Workshop,
                if rng.gen_bool(0.5) { EventVisibility::Public } else { EventVisibility::MembersOnly },
                -days_ago + rng.gen_range(5..15),
                3,
                Some("Training Lab A"),
                admin.id,
            );
            let created_event = event_repo.create(event).await?;
            event_count += 1;

            let attendee_count = rng.gen_range(8..25);
            let mut shuffled: Vec<_> = all_members.iter().collect();
            shuffled.shuffle(&mut rng);
            for (member_id, _) in shuffled.iter().take(attendee_count) {
                let _ = event_repo.register_attendance(created_event.id, *member_id).await;
            }
        }

        // Occasional social
        if month % 3 == 0 {
            let social = social_events.choose(&mut rng).unwrap();
            let event = make_event(
                social,
                &format!("{} - come hang out with fellow members!", social),
                EventType::Social,
                EventVisibility::MembersOnly,
                -days_ago + rng.gen_range(10..20),
                4,
                Some("Member Lounge"),
                admin.id,
            );
            let created_event = event_repo.create(event).await?;
            event_count += 1;

            let attendee_count = rng.gen_range(10..30);
            let mut shuffled: Vec<_> = all_members.iter().collect();
            shuffled.shuffle(&mut rng);
            for (member_id, _) in shuffled.iter().take(attendee_count) {
                let _ = event_repo.register_attendance(created_event.id, *member_id).await;
            }
        }

        // Quarterly CTF
        if month % 3 == 0 {
            let season = match month {
                3 => "Spring",
                6 => "Summer",
                9 => "Fall",
                12 => "Winter",
                _ => "Quarterly",
            };
            let year = (Utc::now() - Duration::days(days_ago)).format("%Y");
            let event = make_event(
                &format!("{} CTF {}", season, year),
                "Capture The Flag competition - test your skills!",
                EventType::CTF,
                EventVisibility::Public,
                -days_ago + 7,
                24,
                Some("Online"),
                admin.id,
            );
            let created_event = event_repo.create(event).await?;
            event_count += 1;

            let attendee_count = rng.gen_range(15..40);
            let mut shuffled: Vec<_> = all_members.iter().collect();
            shuffled.shuffle(&mut rng);
            for (member_id, _) in shuffled.iter().take(attendee_count) {
                let _ = event_repo.register_attendance(created_event.id, *member_id).await;
            }
        }
    }

    // Generate upcoming events (next 3 months)
    for month in 0..3 {
        let days_ahead = month * 30;

        // Monthly meeting
        let event = make_event(
            &format!("Monthly Meeting - {}", (Utc::now() + Duration::days(days_ahead + 7)).format("%B %Y")),
            "Regular monthly meeting to discuss club business and upcoming activities.",
            EventType::Meeting,
            EventVisibility::MembersOnly,
            days_ahead + 7,
            2,
            Some("Main Conference Room"),
            admin.id,
        );
        event_repo.create(event).await?;
        event_count += 1;

        // Upcoming workshop
        let topic = workshop_topics.choose(&mut rng).unwrap();
        let event = make_event(
            topic,
            &format!("Hands-on workshop: {}", topic),
            EventType::Workshop,
            EventVisibility::Public,
            days_ahead + 14,
            3,
            Some("Training Lab A"),
            admin.id,
        );
        event_repo.create(event).await?;
        event_count += 1;
    }

    // One big upcoming CTF
    let event = make_event(
        "Annual CTF Championship",
        "Our biggest CTF of the year! 48 hours of challenges across all categories.",
        EventType::CTF,
        EventVisibility::Public,
        45,
        48,
        Some("Online"),
        admin.id,
    );
    event_repo.create(event).await?;
    event_count += 1;

    println!("    Created {} events with attendance records", event_count);

    // =========================================================================
    // ANNOUNCEMENTS
    // =========================================================================
    println!("📢 Creating announcements...");

    let announcements = [
        ("Welcome to Coterie!", "We're excited to launch our new member management system.", AnnouncementType::News, true, true, 1),
        ("CTF Team Takes First Place!", "Congratulations to our CTF team for winning the regional competition!", AnnouncementType::Achievement, true, false, 5),
        ("New Workshop Series Starting", "Join us for our new security workshop series starting next month.", AnnouncementType::News, true, false, 3),
        ("Dues Reminder", "Friendly reminder that monthly dues are due by the 15th.", AnnouncementType::News, false, false, 10),
        ("Lab Access Update", "New keycard system is now active. See admin for access.", AnnouncementType::News, false, false, 15),
    ];

    for (title, content, ann_type, is_public, featured, days_ago) in &announcements {
        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: title.to_string(),
            content: content.to_string(),
            announcement_type: ann_type.clone(),
            is_public: *is_public,
            featured: *featured,
            published_at: Some(Utc::now() - Duration::days(*days_ago)),
            created_by: admin.id,
            created_at: Utc::now() - Duration::days(*days_ago),
            updated_at: Utc::now() - Duration::days(*days_ago),
        };
        announcement_repo.create(announcement).await?;
    }
    println!("    Created {} announcements", announcements.len());

    // =========================================================================
    // PAYMENTS
    // =========================================================================
    println!("💳 Creating payment records...");
    let mut payment_count = 0;

    // Dues amounts by membership type
    fn dues_amount(mem_type: &MembershipType) -> i64 {
        match mem_type {
            MembershipType::Regular => 5000,    // $50
            MembershipType::Student => 2500,    // $25
            MembershipType::Corporate => 10000, // $100
            MembershipType::Lifetime => 50000,  // $500 one-time
        }
    }

    for (member_id, config) in &all_members {
        match config.membership_type {
            MembershipType::Lifetime => {
                // One-time lifetime payment
                if config.months_active > 0 {
                    let payment = make_payment(
                        *member_id,
                        50000,
                        PaymentStatus::Completed,
                        if rng.gen_bool(0.7) { PaymentMethod::Stripe } else { PaymentMethod::Manual },
                        "Lifetime membership",
                        config.months_active * 30,
                    );
                    payment_repo.create(payment).await?;
                    payment_count += 1;
                }
            }
            _ => {
                if config.bypass_dues {
                    continue; // Honorary members don't pay
                }

                if config.status == MemberStatus::Pending {
                    // Pending payment for new member
                    let payment = make_payment(
                        *member_id,
                        dues_amount(&config.membership_type),
                        PaymentStatus::Pending,
                        PaymentMethod::Stripe,
                        "Initial membership dues",
                        0,
                    );
                    payment_repo.create(payment).await?;
                    payment_count += 1;
                    continue;
                }

                // Monthly payments for the duration of membership
                let months = config.months_active.min(24); // Cap at 2 years
                for month in 0..months {
                    let days_ago = month * 30 + rng.gen_range(1..10);
                    let method = if rng.gen_bool(0.85) { PaymentMethod::Stripe } else { PaymentMethod::Manual };

                    let payment = make_payment(
                        *member_id,
                        dues_amount(&config.membership_type),
                        PaymentStatus::Completed,
                        method,
                        &format!("Monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
                        days_ago,
                    );
                    payment_repo.create(payment).await?;
                    payment_count += 1;
                }

                // Add a failed payment occasionally for expired members
                if config.status == MemberStatus::Expired && rng.gen_bool(0.6) {
                    let payment = make_payment(
                        *member_id,
                        dues_amount(&config.membership_type),
                        PaymentStatus::Failed,
                        PaymentMethod::Stripe,
                        "Monthly dues - Payment declined",
                        rng.gen_range(1..30),
                    );
                    payment_repo.create(payment).await?;
                    payment_count += 1;
                }
            }
        }
    }

    // Add some workshop fees
    let workshop_payers: Vec<_> = all_members.iter()
        .filter(|(_, c)| c.status == MemberStatus::Active)
        .take(15)
        .collect();

    for (member_id, _) in workshop_payers {
        let payment = make_payment(
            *member_id,
            2000, // $20 workshop fee
            PaymentStatus::Completed,
            PaymentMethod::Stripe,
            "Workshop fee",
            rng.gen_range(30..180),
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    // Add a few refunds
    let refund_count = 3;
    let refund_members: Vec<_> = all_members.iter()
        .filter(|(_, c)| c.status == MemberStatus::Active && c.months_active > 6)
        .take(refund_count)
        .collect();

    for (member_id, config) in refund_members {
        let payment = make_payment(
            *member_id,
            dues_amount(&config.membership_type),
            PaymentStatus::Refunded,
            PaymentMethod::Stripe,
            "Monthly dues - Refunded (duplicate charge)",
            rng.gen_range(60..180),
        );
        payment_repo.create(payment).await?;
        payment_count += 1;
    }

    println!("    Created {} payment records", payment_count);

    // =========================================================================
    // SUMMARY
    // =========================================================================
    println!("\n  Database seeding complete!");
    println!("\n📊 Summary:");
    println!("   Members: {} (including admin)", all_members.len() + 1);
    println!("   Events: {}", event_count);
    println!("   Payments: {}", payment_count);
    println!("   Announcements: {}", announcements.len());

    println!("\n📝 Test credentials (password for all: password123):");
    println!("   Admin: admin@coterie.local / admin123");
    println!("");
    println!("   Known test users:");
    println!("     alice@example.com    - Regular, Active");
    println!("     bob@example.com      - Student, Active");
    println!("     charlie@example.com  - Regular, Expired");
    println!("     dave@example.com     - Regular, Pending");
    println!("");
    println!("   Plus {} randomly generated members", generated);

    Ok(())
}
