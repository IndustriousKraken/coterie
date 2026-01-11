use clap::Parser;
use config::{Config, File};
use coterie::{
    domain::{
        CreateMemberRequest, MembershipType, MemberStatus, UpdateMemberRequest,
        CreateEventTypeRequest, CreateAnnouncementTypeRequest, CreateMembershipTypeRequest,
        Event, EventType, EventVisibility,
        Announcement, AnnouncementType,
        Payment, PaymentStatus, PaymentMethod,
    },
    repository::{
        MemberRepository, SqliteMemberRepository,
        EventRepository, SqliteEventRepository,
        AnnouncementRepository, SqliteAnnouncementRepository,
        PaymentRepository, SqlitePaymentRepository,
        EventTypeRepository, SqliteEventTypeRepository,
        AnnouncementTypeRepository, SqliteAnnouncementTypeRepository,
        MembershipTypeRepository, SqliteMembershipTypeRepository,
    },
};
use chrono::{Utc, Duration};
use serde::Deserialize;
use sqlx::sqlite::SqlitePoolOptions;
use std::path::PathBuf;
use uuid::Uuid;
use fake::Fake;
use fake::faker::name::en::{FirstName, LastName};
use rand::Rng;
use rand::seq::SliceRandom;

/// Seed the Coterie database with example data
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Example configuration to use (e.g., "hacker-club", "baduk-club", "congregation")
    #[arg(short, long)]
    example: String,

    /// Number of random members to generate (in addition to test users)
    #[arg(short, long, default_value = "100")]
    member_count: usize,
}

// ============================================================================
// Example Configuration Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct ExampleConfig {
    admin: AdminConfig,
    #[serde(default)]
    test_users: Vec<TestUserConfig>,
    #[serde(default)]
    membership_types: Vec<MembershipTypeConfig>,
    #[serde(default)]
    event_types: Vec<EventTypeConfig>,
    #[serde(default)]
    announcement_types: Vec<AnnouncementTypeConfig>,
    #[serde(default)]
    events: Vec<EventConfig>,
    #[serde(default)]
    announcements: Vec<AnnouncementConfig>,
}

#[derive(Debug, Deserialize)]
struct AdminConfig {
    email: String,
    username: String,
    full_name: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct TestUserConfig {
    email: String,
    username: String,
    full_name: String,
    password: String,
    membership_type: String,
    status: String,
    months_active: i64,
    #[serde(default)]
    bypass_dues: bool,
}

#[derive(Debug, Deserialize)]
struct MembershipTypeConfig {
    name: String,
    slug: String,
    color: String,
    fee_cents: i32,
    billing_frequency: String,
}

#[derive(Debug, Deserialize)]
struct EventTypeConfig {
    name: String,
    slug: String,
    color: String,
    #[serde(default)]
    icon: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnnouncementTypeConfig {
    name: String,
    slug: String,
    color: String,
}

#[derive(Debug, Deserialize)]
struct EventConfig {
    title: String,
    description: String,
    event_type: String,  // slug of the event type
    #[serde(default)]
    days_offset: i64,    // positive = future, negative = past
    #[serde(default = "default_duration")]
    duration_hours: i64,
    #[serde(default)]
    location: Option<String>,
    #[serde(default = "default_visibility")]
    visibility: String,  // "public" or "members_only"
    #[serde(default)]
    max_attendees: Option<i32>,
}

fn default_duration() -> i64 { 2 }
fn default_visibility() -> String { "members_only".to_string() }

#[derive(Debug, Deserialize)]
struct AnnouncementConfig {
    title: String,
    content: String,
    announcement_type: String,  // slug of the announcement type
    #[serde(default)]
    days_ago: i64,
    #[serde(default)]
    is_public: bool,
    #[serde(default)]
    featured: bool,
}

// ============================================================================
// Helper Functions
// ============================================================================

fn parse_membership_type(s: &str) -> MembershipType {
    match s.to_lowercase().as_str() {
        "regular" => MembershipType::Regular,
        "student" => MembershipType::Student,
        "corporate" => MembershipType::Corporate,
        "lifetime" => MembershipType::Lifetime,
        "family" => MembershipType::Regular, // Map family to regular
        "individual" => MembershipType::Regular, // Map individual to regular
        "senior" => MembershipType::Regular, // Map senior to regular
        "founding" => MembershipType::Lifetime, // Map founding to lifetime
        _ => MembershipType::Regular,
    }
}

fn parse_member_status(s: &str) -> MemberStatus {
    match s.to_lowercase().as_str() {
        "active" => MemberStatus::Active,
        "expired" => MemberStatus::Expired,
        "pending" => MemberStatus::Pending,
        "suspended" => MemberStatus::Suspended,
        "honorary" => MemberStatus::Honorary,
        _ => MemberStatus::Pending,
    }
}

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
        event_type_id: None,
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
struct MemberGenConfig {
    status: MemberStatus,
    membership_type: MembershipType,
    months_active: i64,
    bypass_dues: bool,
    notes: Option<String>,
}

fn generate_member_config(rng: &mut impl Rng) -> MemberGenConfig {
    let roll: u8 = rng.gen_range(0..100);

    let (status, membership_type, months_active, bypass_dues, notes) = match roll {
        0..=69 => {
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
            let months = rng.gen_range(3..=12);
            (MemberStatus::Expired, MembershipType::Regular, months, false, None)
        }
        80..=87 => {
            (MemberStatus::Pending, MembershipType::Regular, 0, false, None)
        }
        88..=92 => {
            let months = rng.gen_range(2..=8);
            (MemberStatus::Suspended, MembershipType::Regular, months, false,
             Some("Suspended - under review".to_string()))
        }
        93..=97 => {
            (MemberStatus::Honorary, MembershipType::Regular, 0, true,
             Some("Honorary member".to_string()))
        }
        _ => {
            let months = rng.gen_range(12..=36);
            (MemberStatus::Active, MembershipType::Lifetime, months, true,
             Some("Lifetime member".to_string()))
        }
    };

    MemberGenConfig {
        status,
        membership_type,
        months_active,
        bypass_dues,
        notes,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Load .env file if present
    dotenvy::dotenv().ok();

    // Find and load example config
    let config_path = PathBuf::from(format!("config/examples/{}.toml", args.example));
    if !config_path.exists() {
        eprintln!("Error: Example config not found: {}", config_path.display());
        eprintln!("\nAvailable examples:");
        if let Ok(entries) = std::fs::read_dir("config/examples") {
            for entry in entries.flatten() {
                if let Some(name) = entry.path().file_stem() {
                    eprintln!("  - {}", name.to_string_lossy());
                }
            }
        }
        std::process::exit(1);
    }

    let config: ExampleConfig = Config::builder()
        .add_source(File::from(config_path))
        .build()?
        .try_deserialize()?;

    println!("Seeding database with '{}' example...", args.example);
    println!("   Generating {} members with history", args.member_count);

    let mut rng = rand::thread_rng();

    // Initialize database connection
    let database_url = std::env::var("COTERIE__DATABASE__URL")
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

    // Clear existing data (including defaults from migrations)
    println!("  Clearing any existing data...");
    sqlx::query("DELETE FROM payments").execute(&db_pool).await?;
    sqlx::query("DELETE FROM event_attendance").execute(&db_pool).await?;
    sqlx::query("DELETE FROM events").execute(&db_pool).await?;
    sqlx::query("DELETE FROM announcements").execute(&db_pool).await?;
    sqlx::query("DELETE FROM sessions").execute(&db_pool).await?;
    sqlx::query("DELETE FROM csrf_tokens").execute(&db_pool).await?;
    sqlx::query("DELETE FROM member_profiles").execute(&db_pool).await?;
    sqlx::query("DELETE FROM members").execute(&db_pool).await?;
    // Also clear types so we can seed fresh (migrations create defaults)
    sqlx::query("DELETE FROM event_types").execute(&db_pool).await?;
    sqlx::query("DELETE FROM announcement_types").execute(&db_pool).await?;
    sqlx::query("DELETE FROM membership_types").execute(&db_pool).await?;

    // Initialize repositories
    let member_repo = SqliteMemberRepository::new(db_pool.clone());
    let event_repo = SqliteEventRepository::new(db_pool.clone());
    let announcement_repo = SqliteAnnouncementRepository::new(db_pool.clone());
    let payment_repo = SqlitePaymentRepository::new(db_pool.clone());
    let event_type_repo = SqliteEventTypeRepository::new(db_pool.clone());
    let announcement_type_repo = SqliteAnnouncementTypeRepository::new(db_pool.clone());
    let membership_type_repo = SqliteMembershipTypeRepository::new(db_pool.clone());

    // =========================================================================
    // CONFIGURABLE TYPES
    // =========================================================================
    println!("Creating configurable types...");

    // Seed event types from config
    for et in &config.event_types {
        event_type_repo.create(CreateEventTypeRequest {
            name: et.name.clone(),
            slug: Some(et.slug.clone()),
            description: None,
            color: Some(et.color.clone()),
            icon: et.icon.clone(),
        }).await?;
    }
    println!("    Created {} event types", config.event_types.len());

    // Seed announcement types from config
    for at in &config.announcement_types {
        announcement_type_repo.create(CreateAnnouncementTypeRequest {
            name: at.name.clone(),
            slug: Some(at.slug.clone()),
            description: None,
            color: Some(at.color.clone()),
            icon: None,
        }).await?;
    }
    println!("    Created {} announcement types", config.announcement_types.len());

    // Seed membership types from config
    for mt in &config.membership_types {
        membership_type_repo.create(CreateMembershipTypeRequest {
            name: mt.name.clone(),
            slug: Some(mt.slug.clone()),
            description: None,
            color: Some(mt.color.clone()),
            icon: None,
            fee_cents: mt.fee_cents,
            billing_period: mt.billing_frequency.clone(),
        }).await?;
    }
    println!("    Created {} membership types", config.membership_types.len());

    // =========================================================================
    // MEMBERS
    // =========================================================================
    println!("Creating members...");

    let mut all_members: Vec<(Uuid, MemberGenConfig)> = Vec::new();
    let mut used_usernames: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut used_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Create admin user
    let admin = member_repo.create(CreateMemberRequest {
        email: config.admin.email.clone(),
        username: config.admin.username.clone(),
        full_name: config.admin.full_name.clone(),
        password: config.admin.password.clone(),
        membership_type: MembershipType::Lifetime,
    }).await?;

    member_repo.update(admin.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        notes: Some("ADMIN - System administrator".to_string()),
        bypass_dues: Some(true),
        ..Default::default()
    }).await?;

    used_usernames.insert(config.admin.username.clone());
    used_emails.insert(config.admin.email.clone());
    println!("    Created admin user ({} / {})", config.admin.email, config.admin.password);

    // Create test users from config
    for user_config in &config.test_users {
        let mem_type = parse_membership_type(&user_config.membership_type);
        let status = parse_member_status(&user_config.status);

        let member = member_repo.create(CreateMemberRequest {
            email: user_config.email.clone(),
            username: user_config.username.clone(),
            full_name: user_config.full_name.clone(),
            password: user_config.password.clone(),
            membership_type: mem_type.clone(),
        }).await?;

        let dues_until = if status == MemberStatus::Active {
            Some(Utc::now() + Duration::days(30))
        } else if status == MemberStatus::Expired {
            Some(Utc::now() - Duration::days(45))
        } else {
            None
        };

        let joined = Utc::now() - Duration::days(user_config.months_active * 30);

        sqlx::query("UPDATE members SET status = ?, dues_paid_until = ?, joined_at = ?, bypass_dues = ? WHERE id = ?")
            .bind(format!("{:?}", status))
            .bind(dues_until)
            .bind(joined)
            .bind(user_config.bypass_dues)
            .bind(member.id.to_string())
            .execute(&db_pool)
            .await?;

        all_members.push((member.id, MemberGenConfig {
            status: status.clone(),
            membership_type: mem_type,
            months_active: user_config.months_active,
            bypass_dues: user_config.bypass_dues,
            notes: None,
        }));
        used_usernames.insert(user_config.username.clone());
        used_emails.insert(user_config.email.clone());
    }
    println!("    Created {} test users", config.test_users.len());

    // Generate random members
    let random_count = args.member_count.saturating_sub(1 + config.test_users.len());
    let mut generated = 0;
    let mut attempts = 0;
    const MAX_ATTEMPTS: usize = 1000;

    while generated < random_count && attempts < MAX_ATTEMPTS {
        attempts += 1;

        let first_name: String = FirstName().fake_with_rng(&mut rng);
        let last_name: String = LastName().fake_with_rng(&mut rng);
        let full_name = format!("{} {}", first_name, last_name);

        let mut username = make_username(&first_name, &last_name, &mut rng);
        if used_usernames.contains(&username) {
            username = format!("{}_{}", username, rng.gen_range(100..999));
        }
        if used_usernames.contains(&username) {
            continue;
        }

        let email_domains = ["example.com", "test.local", "mail.test", "demo.org", "sample.net"];
        let domain = email_domains.choose(&mut rng).unwrap();
        let mut email = format!("{}@{}", username, domain);
        if used_emails.contains(&email) {
            email = format!("{}.{}@{}", first_name.to_lowercase(), last_name.to_lowercase(), domain);
        }
        if used_emails.contains(&email) {
            continue;
        }

        let gen_config = generate_member_config(&mut rng);

        let member = member_repo.create(CreateMemberRequest {
            email: email.clone(),
            username: username.clone(),
            full_name,
            password: "password123".to_string(),
            membership_type: gen_config.membership_type.clone(),
        }).await?;

        let months_ago = gen_config.months_active;
        let joined = Utc::now() - Duration::days(months_ago * 30 + rng.gen_range(0..30));

        let dues_until = match gen_config.status {
            MemberStatus::Active => {
                if gen_config.bypass_dues {
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
            .bind(format!("{:?}", gen_config.status))
            .bind(dues_until)
            .bind(joined)
            .bind(gen_config.bypass_dues)
            .bind(&gen_config.notes)
            .bind(member.id.to_string())
            .execute(&db_pool)
            .await?;

        all_members.push((member.id, gen_config));
        used_usernames.insert(username);
        used_emails.insert(email);
        generated += 1;
    }

    println!("    Generated {} random members", generated);

    // =========================================================================
    // EVENTS
    // =========================================================================
    println!("Creating events...");
    let mut event_count = 0;

    // Create events from config
    for event_config in &config.events {
        let visibility = match event_config.visibility.as_str() {
            "public" => EventVisibility::Public,
            _ => EventVisibility::MembersOnly,
        };

        // Map event_type slug to EventType enum (fallback to Meeting)
        let event_type = match event_config.event_type.as_str() {
            "workshop" => EventType::Workshop,
            "social" => EventType::Social,
            "training" => EventType::Training,
            "ctf" => EventType::CTF,
            "hackathon" => EventType::Hackathon,
            _ => EventType::Meeting,
        };

        let event = make_event(
            &event_config.title,
            &event_config.description,
            event_type,
            visibility,
            event_config.days_offset,
            event_config.duration_hours,
            event_config.location.as_deref(),
            admin.id,
        );

        let created_event = event_repo.create(event).await?;
        event_count += 1;

        // Add random attendees to past events
        if event_config.days_offset < 0 {
            let attendee_count = rng.gen_range(8..25);
            let mut shuffled: Vec<_> = all_members.iter().collect();
            shuffled.shuffle(&mut rng);
            for (member_id, _) in shuffled.iter().take(attendee_count) {
                let _ = event_repo.register_attendance(created_event.id, *member_id).await;
            }
        }
    }

    // If no events in config, generate some generic ones
    if config.events.is_empty() {
        // Generate a few monthly meetings
        for month in 0..3 {
            let days_ahead = month * 30 + 7;
            let event = make_event(
                &format!("Monthly Meeting - {}", (Utc::now() + Duration::days(days_ahead)).format("%B %Y")),
                "Monthly gathering to discuss club business and updates.",
                EventType::Meeting,
                EventVisibility::MembersOnly,
                days_ahead,
                2,
                Some("Main Meeting Room"),
                admin.id,
            );
            event_repo.create(event).await?;
            event_count += 1;
        }
    }

    println!("    Created {} events", event_count);

    // =========================================================================
    // ANNOUNCEMENTS
    // =========================================================================
    println!("Creating announcements...");
    let mut announcement_count = 0;

    // Create announcements from config
    for ann_config in &config.announcements {
        // Map announcement_type slug to AnnouncementType enum (fallback to News)
        let ann_type = match ann_config.announcement_type.as_str() {
            "general" => AnnouncementType::General,
            "achievement" => AnnouncementType::Achievement,
            _ => AnnouncementType::News,
        };

        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: ann_config.title.clone(),
            content: ann_config.content.clone(),
            announcement_type: ann_type,
            announcement_type_id: None,
            is_public: ann_config.is_public,
            featured: ann_config.featured,
            published_at: Some(Utc::now() - Duration::days(ann_config.days_ago)),
            created_by: admin.id,
            created_at: Utc::now() - Duration::days(ann_config.days_ago),
            updated_at: Utc::now() - Duration::days(ann_config.days_ago),
        };
        announcement_repo.create(announcement).await?;
        announcement_count += 1;
    }

    // If no announcements in config, create a welcome announcement
    if config.announcements.is_empty() {
        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: "Welcome to Our New Portal!".to_string(),
            content: "We're excited to launch our new member portal. Log in to manage your membership, RSVP to events, and stay connected.".to_string(),
            announcement_type: AnnouncementType::News,
            announcement_type_id: None,
            is_public: true,
            featured: true,
            published_at: Some(Utc::now() - Duration::days(1)),
            created_by: admin.id,
            created_at: Utc::now() - Duration::days(1),
            updated_at: Utc::now() - Duration::days(1),
        };
        announcement_repo.create(announcement).await?;
        announcement_count += 1;
    }

    println!("    Created {} announcements", announcement_count);

    // =========================================================================
    // PAYMENTS
    // =========================================================================
    println!("Creating payment records...");
    let mut payment_count = 0;

    // Get default dues amount from first membership type in config
    let default_dues = config.membership_types.first()
        .map(|mt| mt.fee_cents as i64)
        .unwrap_or(5000);

    for (member_id, gen_config) in &all_members {
        if gen_config.bypass_dues {
            continue;
        }

        if gen_config.status == MemberStatus::Pending {
            let payment = make_payment(
                *member_id,
                default_dues,
                PaymentStatus::Pending,
                PaymentMethod::Stripe,
                "Initial membership dues",
                0,
            );
            payment_repo.create(payment).await?;
            payment_count += 1;
            continue;
        }

        // Monthly payments
        let months = gen_config.months_active.min(24);
        for month in 0..months {
            let days_ago = month * 30 + rng.gen_range(1..10);
            let method = if rng.gen_bool(0.85) { PaymentMethod::Stripe } else { PaymentMethod::Manual };

            let payment = make_payment(
                *member_id,
                default_dues,
                PaymentStatus::Completed,
                method,
                &format!("Monthly dues - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
                days_ago,
            );
            payment_repo.create(payment).await?;
            payment_count += 1;
        }

        // Add failed payment for expired members
        if gen_config.status == MemberStatus::Expired && rng.gen_bool(0.6) {
            let payment = make_payment(
                *member_id,
                default_dues,
                PaymentStatus::Failed,
                PaymentMethod::Stripe,
                "Monthly dues - Payment declined",
                rng.gen_range(1..30),
            );
            payment_repo.create(payment).await?;
            payment_count += 1;
        }
    }

    println!("    Created {} payment records", payment_count);

    // =========================================================================
    // SUMMARY
    // =========================================================================
    println!("\nDatabase seeding complete!");
    println!("\nSummary:");
    println!("   Example: {}", args.example);
    println!("   Members: {} (including admin)", all_members.len() + 1);
    println!("   Events: {}", event_count);
    println!("   Payments: {}", payment_count);
    println!("   Announcements: {}", announcement_count);
    println!("   Event types: {}", config.event_types.len());
    println!("   Announcement types: {}", config.announcement_types.len());
    println!("   Membership types: {}", config.membership_types.len());

    println!("\nCredentials:");
    println!("   Admin: {} / {}", config.admin.email, config.admin.password);
    if !config.test_users.is_empty() {
        println!("\n   Test users:");
        for user in &config.test_users {
            println!("     {} - {}, {}", user.email, user.membership_type, user.status);
        }
    }
    println!("\n   Plus {} randomly generated members (password: password123)", generated);

    Ok(())
}
