# Coterie

Status: Active Development, pre alpha.

## Quick Start

```bash
# Seed the database with test data (optional, clears existing data)
cargo run --bin seed

# Run the server
cargo run --bin coterie
```

**Server runs at**: http://127.0.0.1:8080

### Accessing the System

Coterie serves both a **web portal** (for browsers) and a **JSON API** (for integrations).

| Access Method | What You Get |
|---------------|--------------|
| Browser → `http://127.0.0.1:8080/` | Redirects to login page |
| `curl http://127.0.0.1:8080/` | JSON with API endpoint listing |
| `curl http://127.0.0.1:8080/health` | Health check JSON |

### Web Portal Routes

| Route | Description |
|-------|-------------|
| `/login` | Login page |
| `/portal/dashboard` | Member dashboard |
| `/portal/profile` | Edit profile, change password |
| `/portal/events` | View and RSVP to events |
| `/portal/payments` | Payment history |
| `/portal/admin/members` | Admin: manage members |

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /api` | API info |
| `GET /public/events` | Public events (JSON or iCal) |
| `GET /public/announcements` | Public announcements |
| `GET /public/feed/rss` | RSS feed |
| `GET /public/feed/calendar` | iCal calendar feed |
| `POST /public/signup` | Register new member |
| `GET/POST /api/members` | Member management (auth required) |
| `GET/POST /api/events` | Event management (auth required) |
| `GET/POST /api/payments` | Payment management (auth required) |

### Test Credentials (Development)

| User | Email | Password | Role |
|------|-------|----------|------|
| Admin | admin@coterie.local | admin123 | Admin |
| Alice | alice@example.com | password123 | Active member |
| Bob | bob@example.com | password123 | Active student |
| Charlie | charlie@example.com | password123 | Expired |
| Dave | dave@example.com | password123 | Pending |

Coterie is a secure, lightweight member management system designed for small to medium-sized groups, clubs, and organizations. Built with security and maintainability in mind, it provides a simple yet powerful platform for managing memberships without the complexity of enterprise solutions.

## Overview

Coterie is a member management system for clubs, groups, social organizations etc. 
You can connect it to your website to verify dues payments and register new members, 
and for members to self-service their accounts. Admins can use Coterie to check 
member status, activate/approve memberships, and update member details.

At its core, Coterie strives to do one thing very well: to make sure you know who is in your group, and who is not.

## Architecture

Coterie uses a **dual-frontend architecture** to separate public-facing content from member management:

```
┌─────────────────────┐         ┌──────────────────────┐
│  Public Website     │         │  Management Portal   │
│  (Static Site)      │         │  (HTMX + Alpine.js)  │
├─────────────────────┤         ├──────────────────────┤
│ • Marketing pages   │         │ • Member dashboard   │
│ • Event calendar    │         │ • Admin panel        │
│ • Announcements     │         │ • Payment management │
│ • Signup form       │         │ • Profile editing    │
│ • Member directory  │         │ • Event RSVP         │
└──────────┬──────────┘         └──────────┬───────────┘
           │                                │
           ▼                                ▼
     Public APIs                     Protected APIs
           │                                │
           └────────────┬───────────────────┘
                        │
                 ┌──────▼──────┐
                 │   Coterie   │
                 │   Backend   │
                 └─────────────┘
```

- **Public Website**: Your existing website (built with any technology) consumes Coterie's public APIs to display events, announcements, and handle signups
- **Management Portal**: Built-in admin and member interface served by Coterie for account management
- **Coterie Backend**: Single Rust binary providing both public and authenticated APIs

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed integration examples.

## Technology Stack

- **Backend**: Rust (using Axum web framework)
- **Database**: SQLite with WAL mode
- **Management Portal**: HTMX + Alpine.js for minimal, secure interfaces
- **Public Website**: Any static site generator or framework (your choice)
- **Authentication**: Session-based with secure cookies, Argon2id for password hashing, TOTP for 2FA
- **Deployment**: Single binary deployment with Caddy reverse proxy

## Core Features

### Currently Planned
- **Member Management**: Track active, expired, and pending members
- **Payment Integration**: Stripe integration for dues (no card details stored)
- **Public API**: For member signup and verification from static websites
- **Admin Dashboard**: Manage members, view audit logs, configure settings
- **Calendar System**: Manage events with public/member-only visibility
- **Public Achievements**: Display meeting info, CTF results, member accomplishments
- **RSS Feeds**: For public announcements and member blog aggregation

### Integration System
Coterie uses a modular plugin architecture for third-party integrations:
- **Discord**: Automatically manage member roles based on dues status
- **Unifi Access**: Grant/revoke physical access to facilities
- **VPN/Network**: Manage WireGuard VPN access for lab resources

### Planned Features
- **Expense Tracking**: Track and report group expenses with transparency reports
- **Member Directory**: Opt-in skills/expertise directory
- **Resource Library**: Share tools, guides, and writeups with access controls
- **Audit Logging**: Complete trail of all administrative actions
