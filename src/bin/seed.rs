use coterie::{
    config::Settings,
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

// Helper functions to convert string types from config
fn parse_membership_type(s: &str) -> MembershipType {
    match s.to_lowercase().as_str() {
        "regular" | "monthly_regular" => MembershipType::Regular,
        "student" => MembershipType::Student,
        "corporate" => MembershipType::Corporate,
        "lifetime" => MembershipType::Lifetime,
        _ => MembershipType::Regular,
    }
}

fn parse_member_status(s: &str) -> MemberStatus {
    match s.to_lowercase().as_str() {
        "active" => MemberStatus::Active,
        "expired" => MemberStatus::Expired,
        "pending" => MemberStatus::Pending,
        _ => MemberStatus::Pending,
    }
}

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
    
    // Load configuration
    let settings = Settings::new().unwrap_or_else(|e| {
        println!("⚠️  Failed to load config: {}. Using defaults.", e);
        Settings::default()
    });
    
    println!("   Generating {} members with {} months of history", MEMBER_COUNT, MONTHS_OF_HISTORY);

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
    let admin_config = &settings.seed.admin;
    let admin = member_repo.create(CreateMemberRequest {
        email: admin_config.email.clone(),
        username: admin_config.username.clone(),
        full_name: admin_config.full_name.clone(),
        password: admin_config.password.clone(),
        membership_type: MembershipType::Lifetime,
    }).await?;

    member_repo.update(admin.id, UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        notes: Some("ADMIN - System administrator".to_string()),
        bypass_dues: Some(true),
        ..Default::default()
    }).await?;

    used_usernames.insert(admin_config.username.clone());
    used_emails.insert(admin_config.email.clone());
    println!("    Created admin user ({} / {})", admin_config.email, admin_config.password);

    // Create the test users from configuration
    for user_config in &settings.seed.test_users {
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

        sqlx::query("UPDATE members SET status = ?, dues_paid_until = ?, joined_at = ?, bypass_dues = ?, notes = ? WHERE id = ?")
            .bind(format!("{:?}", status))
            .bind(dues_until)
            .bind(joined)
            .bind(user_config.bypass_dues)
            .bind(&user_config.notes)
            .bind(member.id.to_string())
            .execute(&db_pool)
            .await?;

        all_members.push((member.id, MemberConfig {
            status: status.clone(),
            membership_type: mem_type,
            months_active: user_config.months_active,
            bypass_dues: user_config.bypass_dues,
            notes: user_config.notes.clone(),
        }));
        used_usernames.insert(user_config.username.clone());
        used_emails.insert(user_config.email.clone());
    }
    println!("    Created {} test users", settings.seed.test_users.len());

    // Generate random members
    let test_users_count = settings.seed.test_users.len();
    let random_count = MEMBER_COUNT.saturating_sub(1 + test_users_count); // admin + test users
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

    // Hipster Electronics Hacking Club themed events
    let workshop_events: Vec<(&str, &str)> = vec![
        ("Restoring a TRS-80 Model I", r#"Join us for a hands-on workshop where we'll restore a TRS-80 Model I from a dusty garage find to a fully functional piece of computing history.

We'll cover disassembly, cleaning decades of grime from the keyboard mechanism, replacing failing capacitors on the power supply, and troubleshooting the video output circuit. Bring your own multimeter if you have one.

The TRS-80 was released in 1977 and represented one of the first mass-market personal computers. Understanding its architecture gives us insight into the foundations of modern computing - when 4KB of RAM was considered luxurious and programmers had to squeeze every last byte.

No prior experience required, but basic soldering skills are helpful. We'll have loaner equipment available.

Snacks provided. Please RSVP so we know how many pizza boxes to order."#),

        ("BASIC Programming on Commodore 64", r#"Remember when programming meant typing in listings from magazines and hoping you didn't make a typo on line 2340? Let's relive those days together.

In this workshop, we'll cover the fundamentals of Commodore BASIC 2.0, from simple PRINT statements to sprite graphics and SID chip sound programming. We'll be using real hardware - no emulators allowed (okay, maybe one emulator for the projector).

Topics covered:
- Memory layout and PEEK/POKE addresses you should memorize
- Creating simple games with character graphics
- Introduction to machine language via SYS calls
- Why line numbers were actually a reasonable design choice (fight me)

Bring your own C64 if you have one, or use one of the club's machines. We have several breadbin models and one pristine C64C that we only bring out for special occasions.

This workshop is suitable for beginners who want to understand vintage computing, as well as grizzled veterans who want to relive their youth and complain about how kids today don't appreciate real programming."#),

        ("Z80 Assembly Language Fundamentals", r#"Before there was x86, before ARM dominated mobile, there was the Zilog Z80. This workshop is your gateway to understanding assembly language on one of the most elegant 8-bit processors ever designed.

We'll start with the basics: registers, flags, and the fetch-decode-execute cycle. Then we'll write real programs for CP/M systems, covering:

- Loading values and basic arithmetic
- Memory addressing modes (the Z80 has some clever ones)
- Loop constructs and conditional branching
- Stack operations and subroutines
- Interfacing with the BDOS for file I/O

By the end of the session, you'll have written a complete program that runs on actual vintage hardware. We'll be using a Kaypro II for demonstrations, but the concepts apply to any Z80-based system.

Prerequisites: Some programming experience in any language. An appreciation for doing things the hard way. Willingness to count in hexadecimal.

The Z80 instruction set will make you a better programmer, even if you never touch assembly again. Understanding what the machine is actually doing removes the mysticism from higher-level languages."#),

        ("Building a 6502 Computer from Scratch", r#"The 6502 processor powered the Apple II, Commodore 64, NES, and countless other systems. In this multi-session workshop, we'll build a working computer around this legendary chip.

Session 1 covers the CPU itself: clock generation, reset circuit, and getting the processor to execute its first instruction. We'll use LEDs on the address bus to watch the 6502 fetch bytes - it's mesmerizing once you understand what you're seeing.

By the end of the series, you'll have:
- A working single-board computer
- 32KB of RAM and space for ROM
- Serial I/O for terminal communication
- The knowledge to expand it further

This project is inspired by Ben Eater's excellent 6502 series, but we'll be using through-hole components for easier soldering and debugging. Total parts cost is around $50-75 depending on what you already have in your parts bin.

Soldering experience required. You should be comfortable with a breadboard and reading schematics. We'll go slow, but this isn't an introduction to electronics."#),

        ("Vintage CRT Monitor Repair Safety", r#"CRT monitors contain potentially lethal voltages even when unplugged. This workshop covers safe handling, discharge procedures, and common repair techniques for keeping your vintage displays alive.

This is NOT a workshop where we encourage you to poke around inside CRTs casually. The goal is to teach you the proper safety procedures so that IF you decide to attempt repairs, you do so without killing yourself.

We'll cover:
- How CRTs work and why they hold a charge
- Proper discharge procedures using appropriate tools
- Common failure modes: flyback transformers, capacitors, yoke issues
- When to repair vs. when to recycle responsibly
- Building a discharge tool and why you shouldn't skip this step

Our guest presenter has 30 years of experience repairing arcade monitors and televisions. He has the scars to prove why safety matters.

Space is limited to 15 participants for this hands-on session. Long pants and closed-toe shoes required. We'll provide safety glasses."#),

        ("Retrocomputing Networking: Getting Your Vintage Machines Online", r#"Your Apple IIe doesn't need to be an island. This workshop explores options for connecting vintage computers to modern networks and even the internet.

We'll demonstrate several approaches across different platforms:

For Apple II: Uthernet II cards, serial-to-WiFi bridges, and the ADTPro protocol for disk transfers.

For Commodore: The increasingly popular WiFi modems that emulate dial-up BBSes, plus Ethernet cartridges for the C64.

For CP/M machines: Serial connections through Raspberry Pi bridges, and native TCP/IP stacks that run on Z80.

The internet in 2024 is hostile to 8-bit machines, so we'll also cover setting up local BBSes, Gopher servers, and other protocols from a more civilized age. There's a thriving retro-internet community out there.

This workshop requires some existing familiarity with your vintage platform of choice. Bring your machine if it's portable, or use one of ours for the hands-on portions."#),

        ("Mechanical Keyboard Building: Cherry MX to Alps", r#"Modern mechanical keyboards owe everything to vintage designs. In this workshop, we'll build custom keyboards while exploring the history from IBM Model M buckling springs to Alps switches to modern Cherry MX variants.

Each participant will build a 40% keyboard kit during the session (kit cost: $45, included in workshop fee). We'll cover:

- Switch technology: linear, tactile, clicky - what's actually happening inside
- Keycap profiles and materials: why vintage doubleshot ABS commands premium prices
- PCB vs. hand-wiring tradeoffs
- QMK firmware basics for custom layouts
- Where to find vintage parts and what's worth harvesting

We'll have several vintage keyboards available for examination, including an IBM Model F, original Alps boards from the 80s, and a genuine Space Cadet keyboard (no, you can't use it, just look).

Soldering experience required - you'll be doing about 50 solder joints. Bring your own iron if you prefer, otherwise club equipment is available."#),

        ("Oscilloscope Fundamentals for Retrocomputing", r#"An oscilloscope is the most powerful debugging tool you can have on your bench. This workshop teaches you to use one effectively for vintage computer repair and homebrew projects.

We'll start with basic operation: triggering, timebase, voltage scales. Then move into practical applications:

- Checking clock signals and timing
- Debugging bus issues by watching address and data lines
- Analyzing serial protocols
- Finding bad connections and marginal signals
- Using the scope as a simple logic analyzer

The club has several analog scopes and one digital storage scope available. We encourage you to bring your own if you have one - learning on your own equipment is best.

No prior experience required, but basic electronics knowledge is helpful. By the end, you'll be comfortable connecting probes to a circuit board without worrying about breaking something.

Time permitting, we'll also cover building a simple probe station for SOC packages - useful for anyone working with later-era vintage hardware."#),
    ];

    let meeting_descriptions: Vec<&str> = vec![
        "Monthly meeting to discuss club business, show off projects, and enjoy some 8-bit camaraderie. We'll have our usual show-and-tell segment where members can demonstrate what they've been working on. New members especially welcome - we don't bite, unless you suggest emulation is just as good as real hardware.",
        "Regular gathering of the HEHC faithful. This month's agenda includes planning for the upcoming retrocomputing fair, voting on new equipment purchases for the lab, and a special presentation on recently acquired Apple Lisa documentation.",
        "Our monthly get-together. Main topic this month: organizing the club's growing collection of vintage software. We need volunteers for cataloging and imaging disks before they succumb to bit rot. Also: someone found a working Amiga 4000 at a garage sale and will be showing it off.",
    ];

    let social_events: Vec<(&str, &str)> = vec![
        ("Retro Gaming Night: Console Wars Edition", "Settle the eternal debates once and for all. NES vs. Master System. Genesis vs. SNES. We'll have tournaments, casual play, and heated arguments about blast processing. Bring your own controllers if you're particular about d-pad feel."),
        ("WarGames Movie Night", "Shall we play a game? Join us for a screening of the 1983 classic, followed by discussion of what the movie got right and wrong about computing. Popcorn and period-appropriate snacks provided. We promise not to rant about the WOPR's blinking lights for more than 20 minutes."),
        ("Vintage Computer Swap Meet", "Clean out your closet, expand your collection. Tables available for members wanting to sell or trade equipment. All vintage computing items welcome. Please, no beige PCs from the Windows XP era - we have standards."),
        ("Summer Soldering Social", "Bring a project, any project. Work on your builds while socializing with fellow enthusiasts. We'll have extra bench space set up and plenty of flux. Someone usually brings homebrew beer and period-appropriate snacks."),
        ("Holiday Potluck & Demo Party", "End-of-year celebration with food, drinks, and demos running on vintage hardware. We'll have a small competition for best demo - categories for different platforms. Prizes are bragging rights and possibly some donor hardware."),
    ];

    let hackathon_events: Vec<(&str, &str)> = vec![
        ("48-Hour Game Jam: 8-Bit Edition", r#"Create a game for any 8-bit platform in 48 hours. This isn't a competition for the faint of heart - you'll need to know your target platform well enough to be productive under pressure.

Allowed platforms: Apple II, C64, Atari 8-bit, ZX Spectrum, MSX, NES, or any other 8-bit home computer or console. Emulators allowed for development, but final submission must run on real hardware (we'll test).

Teams of 1-3 people. Original code only - no game engines or existing frameworks (assemblers and compilers are fine).

Judging criteria: gameplay, technical achievement, polish, and adherence to platform conventions. A beautiful game that would have been impossible in 1985 will score lower than a modest game that fits the era.

Previous themes have included "gravity," "one button," and "forgotten." Theme announced at kickoff. Pizza provided for participants who stay overnight."#),

        ("Restore-a-thon: Bulk Lot Challenge", r#"We've acquired a lot of 20 "untested" computers from an estate sale. Some work, some don't, some are missing parts. Your mission: get as many running as possible in 24 hours.

Teams will be randomly assigned to machines. Scoring based on:
- Machine boots to prompt: 1 point
- Machine passes memory test: +1 point
- Machine loads software from storage: +1 point
- Machine was previously non-functional: x2 multiplier

We'll have a shared parts bin and repair station. Teams may trade parts and information. The goal is more running computers, not hoarding.

This is chaotic, educational fun. You'll see failure modes you've never encountered, and probably fix things in ways that shouldn't work but do.

Participants keep nothing - all working machines go into the club's loaner library or are donated to schools. But you'll leave with stories and skills."#),
    ];

    // Generate past events (last 12 months)
    let mut workshop_idx = 0;
    for month in 1..=12 {
        let days_ago = month * 30;

        // Monthly meeting - every month
        let meeting_desc = meeting_descriptions[month as usize % meeting_descriptions.len()];
        let event = make_event(
            &format!("Monthly Meeting - {}", (Utc::now() - Duration::days(days_ago)).format("%B %Y")),
            meeting_desc,
            EventType::Meeting,
            EventVisibility::MembersOnly,
            -days_ago,
            2,
            Some("The Vintage Vault"),
            admin.id,
        );
        let created_event = event_repo.create(event).await?;
        event_count += 1;

        // Add random attendees
        let attendee_count = rng.gen_range(8..25);
        let mut shuffled: Vec<_> = all_members.iter().collect();
        shuffled.shuffle(&mut rng);
        for (member_id, _) in shuffled.iter().take(attendee_count) {
            let _ = event_repo.register_attendance(created_event.id, *member_id).await;
        }

        // Workshop every month (cycling through our detailed workshops)
        let (workshop_title, workshop_desc) = &workshop_events[workshop_idx % workshop_events.len()];
        workshop_idx += 1;
        let event = make_event(
            workshop_title,
            workshop_desc,
            EventType::Workshop,
            if rng.gen_bool(0.7) { EventVisibility::Public } else { EventVisibility::MembersOnly },
            -days_ago + rng.gen_range(7..20),
            3,
            Some("Hardware Lab"),
            admin.id,
        );
        let created_event = event_repo.create(event).await?;
        event_count += 1;

        let attendee_count = rng.gen_range(10..20);
        let mut shuffled: Vec<_> = all_members.iter().collect();
        shuffled.shuffle(&mut rng);
        for (member_id, _) in shuffled.iter().take(attendee_count) {
            let _ = event_repo.register_attendance(created_event.id, *member_id).await;
        }

        // Social event every other month
        if month % 2 == 0 {
            let (social_title, social_desc) = social_events.choose(&mut rng).unwrap();
            let event = make_event(
                social_title,
                social_desc,
                EventType::Social,
                EventVisibility::MembersOnly,
                -days_ago + rng.gen_range(10..25),
                4,
                Some("The Vintage Vault"),
                admin.id,
            );
            let created_event = event_repo.create(event).await?;
            event_count += 1;

            let attendee_count = rng.gen_range(15..35);
            let mut shuffled: Vec<_> = all_members.iter().collect();
            shuffled.shuffle(&mut rng);
            for (member_id, _) in shuffled.iter().take(attendee_count) {
                let _ = event_repo.register_attendance(created_event.id, *member_id).await;
            }
        }

        // Hackathon twice a year
        if month == 6 || month == 12 {
            let (hack_title, hack_desc) = hackathon_events.choose(&mut rng).unwrap();
            let event = make_event(
                hack_title,
                hack_desc,
                EventType::Hackathon,
                EventVisibility::Public,
                -days_ago + 14,
                48,
                Some("The Vintage Vault"),
                admin.id,
            );
            let created_event = event_repo.create(event).await?;
            event_count += 1;

            let attendee_count = rng.gen_range(20..40);
            let mut shuffled: Vec<_> = all_members.iter().collect();
            shuffled.shuffle(&mut rng);
            for (member_id, _) in shuffled.iter().take(attendee_count) {
                let _ = event_repo.register_attendance(created_event.id, *member_id).await;
            }
        }
    }

    // Generate upcoming events (next 4 months)
    for month in 0..4 {
        let days_ahead = month * 30;

        // Monthly meeting
        let meeting_desc = meeting_descriptions[month as usize % meeting_descriptions.len()];
        let event = make_event(
            &format!("Monthly Meeting - {}", (Utc::now() + Duration::days(days_ahead + 7)).format("%B %Y")),
            meeting_desc,
            EventType::Meeting,
            EventVisibility::MembersOnly,
            days_ahead + 7,
            2,
            Some("The Vintage Vault"),
            admin.id,
        );
        event_repo.create(event).await?;
        event_count += 1;

        // Upcoming workshop with detailed description
        let (workshop_title, workshop_desc) = &workshop_events[workshop_idx % workshop_events.len()];
        workshop_idx += 1;
        let event = make_event(
            workshop_title,
            workshop_desc,
            EventType::Workshop,
            EventVisibility::Public,
            days_ahead + 14,
            3,
            Some("Hardware Lab"),
            admin.id,
        );
        event_repo.create(event).await?;
        event_count += 1;
    }

    // Upcoming social events
    let event = make_event(
        "Retro Gaming Night: Console Wars Edition",
        "Settle the eternal debates once and for all. NES vs. Master System. Genesis vs. SNES. We'll have tournaments, casual play, and heated arguments about blast processing. Bring your own controllers if you're particular about d-pad feel. Pizza and drinks provided.",
        EventType::Social,
        EventVisibility::MembersOnly,
        21,
        5,
        Some("The Vintage Vault"),
        admin.id,
    );
    event_repo.create(event).await?;
    event_count += 1;

    // Big upcoming hackathon with long description
    let event = make_event(
        "VintageHack 2026: 48-Hour Retrocomputing Challenge",
        r#"Our flagship annual event returns! VintageHack is a 48-hour hackathon dedicated to creating new software and hardware for vintage computing platforms.

This year's theme: "Connectivity" - build something that helps vintage computers communicate, whether with each other, with modern systems, or with the wider world.

Categories:
- Best New Software: Create an application, game, or utility for any pre-1995 platform
- Best Hardware Project: Build an interface, expansion, or peripheral
- Best Preservation Project: Tools for archiving, emulating, or documenting vintage systems
- People's Choice: Voted by all attendees

Rules and Guidelines:
All projects must run on or interface with hardware designed before 1995. Modern tools may be used for development, but the end result must work on vintage hardware. Teams of 1-4 people. You may start with existing code/designs but must make substantial additions.

Schedule:
Friday 6pm: Kickoff, theme announcement, team formation
Saturday: Hacking continues, midnight snack run
Sunday: Hacking continues, demos begin at 4pm
Sunday 6pm: Judging and awards

Prizes include vintage hardware from our collection, including a working Amiga 2000, a boxed Apple IIc, and various peripherals and software.

Registration includes all meals (pizza Friday, BBQ Saturday, pizza again Sunday), snacks, and beverages. Sleeping arrangements are BYOB (bring your own beanbag) - we'll have the vault open all night but no formal sleeping accommodations.

Space is limited to 40 participants. Members get priority registration until two weeks before the event."#,
        EventType::Hackathon,
        EventVisibility::Public,
        60,
        48,
        Some("The Vintage Vault"),
        admin.id,
    );
    event_repo.create(event).await?;
    event_count += 1;

    // Add a training event
    let event = make_event(
        "New Member Orientation: Introduction to the Lab",
        r#"Required session for new members who want lab access. We'll cover safety procedures, equipment usage, and club policies.

Topics include:
- Lab hours and access procedures
- Soldering station usage and safety
- Oscilloscope and multimeter basics
- Where to find documentation and manuals
- How to check out equipment
- Etiquette and cleanup expectations

This session is mandatory before you can use lab equipment unsupervised. We run it monthly, so don't worry if you can't make this one.

Existing members are welcome to attend as refreshers or to help answer questions. We're always looking for volunteers to help with orientation sessions."#,
        EventType::Training,
        EventVisibility::MembersOnly,
        10,
        2,
        Some("Hardware Lab"),
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
        ("Welcome to the Hipster Electronics Hacking Club!", "We're excited to launch our new member portal! Log in to manage your membership, RSVP to events, and connect with fellow vintage computing enthusiasts. If you have any issues, please contact the admin team.", AnnouncementType::News, true, true, 1),
        ("Apple IIe Donation - Thank You!", "A generous donor has contributed a working Apple IIe with dual floppy drives and a monitor to our collection. The system has been cleaned, tested, and is now available in the lab. Special thanks to the donor who wishes to remain anonymous (but we know it was someone who appreciates proper disk drive alignment).", AnnouncementType::Achievement, true, false, 5),
        ("New Workshop Series: From Bits to Bytes", "We're launching a new workshop series covering computer fundamentals from the ground up. Starting with basic logic gates and progressing through CPU architecture. Perfect for members who want to understand what's actually happening inside those vintage machines. First session is next month - see the events calendar.", AnnouncementType::News, true, false, 3),
        ("Dues Reminder for January", "Friendly reminder that monthly dues are due by the 15th. You can pay through the member portal or at any meeting. Students, remember to bring your current student ID if you haven't already verified your student status this semester.", AnnouncementType::News, false, false, 10),
        ("Lab Hours Extended", "Good news! Starting this month, lab hours are extended to 10pm on Thursdays. Key card access remains available 24/7 for members who have completed the lab orientation. Please remember to log your usage in the sign-in book.", AnnouncementType::News, false, false, 15),
        ("VintageHack 2026 Registration Open", "Registration is now open for VintageHack 2026, our annual 48-hour hackathon. This year's theme will be announced at kickoff, but start thinking about what vintage platform you want to target. Members get priority registration until two weeks before the event. Space is limited to 40 participants.", AnnouncementType::News, true, true, 2),
        ("Found: Mystery Expansion Card", "Someone left an unmarked ISA expansion card in the lab last week. It appears to be some kind of I/O controller but we haven't been able to identify it. If it's yours, please claim it at the next meeting. If you can identify what it is, we'd love to know - bonus points if you can find documentation.", AnnouncementType::News, false, false, 8),
    ];

    for (title, content, ann_type, is_public, featured, days_ago) in &announcements {
        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: title.to_string(),
            content: content.to_string(),
            announcement_type: ann_type.clone(),
            announcement_type_id: None,
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

    println!("\n📝 Test credentials:");
    println!("   Admin: {} / {}", settings.seed.admin.email, settings.seed.admin.password);
    println!("");
    println!("   Test users:");
    for user_config in &settings.seed.test_users {
        println!("     {} - {}, {}", user_config.email, user_config.membership_type, user_config.status);
    }
    println!("");
    println!("   Plus {} randomly generated members", generated);

    Ok(())
}
