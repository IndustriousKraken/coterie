# Coterie Development TODO

## Phase 1: Core Implementation (Priority: High)

### Database & Repository Layer
- [x] Implement SQLite repository for Members
  - [x] CRUD operations
  - [x] Search and filtering (by email, username, id)
  - [x] Password hashing integration (Argon2id)
- [x] Implement SQLite repository for Events
  - [x] CRUD operations with proper type conversions
  - [x] Attendance registration/cancellation
- [x] Implement SQLite repository for Announcements
  - [x] CRUD operations with visibility control
  - [x] List by recent, public, featured
- [x] Implement SQLite repository for Payments
  - [x] CRUD operations with status tracking
  - [x] Find by member and Stripe ID
- [x] Add database connection pooling and error handling
- [x] Create database seeding script for development (`cargo run --bin seed`)

### Authentication & Authorization
- [x] Implement session management
  - [x] Session creation/validation
  - [x] Secure cookie handling
  - [x] Session expiry and cleanup
- [x] Implement authentication middleware
  - [x] Password verification
  - [x] Session validation
  - [x] Role-based access (member/admin)
- [x] CSRF protection for state-changing requests
- [ ] Add TOTP/2FA support
- [ ] Implement password reset flow

### API Handlers
- [x] Implement member management handlers
  - [x] Create, read, update, delete
  - [x] Activation/expiration logic
  - [ ] Profile management
- [x] Implement event management handlers
  - [x] CRUD operations
  - [x] RSVP/attendance tracking
  - [x] iCal feed generation
- [x] Implement announcement handlers
  - [x] CRUD with visibility controls
  - [x] RSS feed generation
- [x] Implement public API endpoints
  - [x] Member signup
  - [x] Public event listing
  - [x] Public announcements

### Testing
- [ ] Unit tests for domain logic
- [x] Integration tests for repositories (Member repository tested)
- [ ] API endpoint tests
- [x] Authentication/authorization tests (Basic auth tested)

## Phase 2: Payment Integration (Priority: High)

### Stripe Integration
- [x] Implement Stripe client wrapper
- [x] Create payment initiation flow
- [x] Handle webhook callbacks
- [x] Implement payment status synchronization
- [ ] Add subscription management
- [x] Create payment history views

### Member Dues Management
- [ ] Automated expiration checking (cron job)
- [ ] Grace period handling
- [ ] Payment reminder system
- [x] Manual payment recording for cash/check

## Phase 3: Frontend (Priority: Medium)

### Admin Dashboard
- [x] Create HTMX base template
- [x] Member management interface
  - [x] List/search members with filtering
  - [x] Activate/suspend members
  - [x] View member details page
  - [x] Edit member details
  - [x] Add new member
  - [x] Manual dues expiration/extension
- [x] Event management interface
  - [x] List/search events with filtering (type, visibility, time)
  - [x] Sortable columns
  - [x] Create new event
  - [x] Edit event details
  - [x] Delete event
  - [ ] Recurring events
    - [ ] Recurrence patterns (daily, weekly, monthly, yearly)
    - [ ] Custom patterns (e.g., "2nd Wednesday", "every other week")
    - [ ] Repeat count (e.g., "for 10 occurrences") or end date
    - [ ] Repeat forever option
    - [ ] Edit single vs. all future occurrences
    - [ ] Cancel single occurrence without deleting series
  - [x] User-definable event types
    - [x] Admin interface to create/edit event types
    - [x] Custom colors/icons for event types
    - [x] Replace hardcoded EventType enum with database table
    - [x] Make all types deletable (removed is_system restriction)
- [x] Announcement editor
  - [x] List/search with filtering (type, status)
  - [x] Create/edit/delete announcements
  - [x] Publish/unpublish workflow
  - [ ] Announcement distribution
    - [ ] RSS feed for public announcements (frontend)
    - [ ] Push to Discord channel on publish
    - [ ] Scheduled delivery to chat (publish now vs. schedule for later)
    - [ ] Support for other chat APIs (Slack, Matrix, etc.)
- [ ] Payment history view
- [ ] Audit log viewer

### Member Portal
- [x] Login/logout pages
- [x] Member dashboard with status, events, payments
- [x] Profile management (view/edit name, change password)
- [x] Events listing page with filtering
- [x] Payment history page with summary
- [ ] Event RSVP functionality
- [ ] Download receipts

### Public Pages
- [ ] Signup form
- [ ] Event calendar
- [ ] Public announcements
- [ ] Member directory (opt-in)

## Phase 4: Integrations (Priority: Medium)

### Discord Integration
- [ ] Implement Discord bot with serenity/twilight
- [ ] Member role management
- [ ] Expired member room handling
- [ ] Sync member profiles
- [ ] Command interface for checking member status

### Unifi Integration
- [ ] Implement Unifi API client
- [ ] Access card provisioning
- [ ] VPN user management
- [ ] Access revocation on expiry
- [ ] Sync scheduling

### Calendar Integration
- [ ] Google Calendar sync
- [ ] Office 365 calendar sync
- [ ] CalDAV support

## Phase 5: Extended Features (Priority: Low)

### Expense Tracking
- [ ] Expense entry interface
- [ ] Receipt upload and storage
- [ ] Category management
- [ ] Quarterly report generation
- [ ] Public transparency dashboard

### Member Features
- [ ] Skills directory
- [ ] Blog aggregation from RSS feeds
- [ ] Achievement badges
- [ ] Equipment checkout system
- [ ] Voting/polls system

### Communication
- [ ] Email notification system
- [ ] Announcement digests
- [ ] Event reminders
- [ ] Welcome emails for new members

### Advanced Admin Features
- [ ] Bulk member import/export
- [ ] Custom fields for members
- [ ] Report builder
- [ ] Backup and restore tools
- [ ] Multi-tenant support (for other groups using the software)

## Phase 6: Operations (Priority: Medium)

### Deployment & Operations
- [ ] Docker containerization
- [ ] SystemD service files
- [ ] Caddy configuration examples
- [ ] Backup scripts
- [ ] Monitoring and alerting setup
- [ ] Rate limiting implementation

### Documentation
- [ ] API documentation (OpenAPI/Swagger)
- [ ] Administrator guide
- [ ] Installation guide
- [ ] Contributing guidelines
- [ ] Security policy

### Performance & Security
- [ ] Security audit
- [ ] Performance profiling
- [ ] Database query optimization
- [ ] Caching strategy (Redis optional)
- [ ] GDPR compliance tools

## Development Environment

### Tooling
- [ ] Development container setup
- [ ] Pre-commit hooks
- [ ] CI/CD pipeline (GitHub Actions)
- [ ] Database migration tooling

### First-Run Setup & Seed Restructuring
- [x] First-run setup flow
  - [x] Middleware to detect fresh database (no admin users)
  - [x] Redirect to /setup when no admin exists
  - [x] Setup page: org name, admin email/username/password
  - [x] Create admin user and redirect to login
- [x] Restructure seed configs
  - [x] Move config/seed.toml to config/examples/hacker-club.toml
  - [x] Create config/examples/baduk-club.toml
  - [x] Create config/examples/congregation.toml
- [x] Update seed binary
  - [x] Require --example flag (no default seed)
  - [x] Load example config from config/examples/<name>.toml
  - [x] Parse event_types and announcement_types from config

### Seed Data Improvements
- [x] Basic seed script with test accounts
- [x] Realistic payment history (monthly dues over 1+ years per member)
- [x] More calendar events (variety of types, past and upcoming)

## Notes

- **Current Status**: Core functionality complete. Admin portal fully functional with member, event, and announcement management. Configurable types (event, announcement, membership) fully implemented with admin UI.
- **Next Step**: RSVP functionality, public pages, announcement distribution
- **Blocking Issues**: None currently
- **Dependencies**: Need to evaluate specific Discord and Unifi API libraries
- **Recently Completed**:
  - ✅ Seed config restructuring (config/examples/ with hacker-club, baduk-club, congregation)
  - ✅ Seed binary now requires --example flag, parses all type configs
  - ✅ First-run setup flow (middleware redirects to /setup, creates admin user)
  - ✅ Configurable types system (event types, announcement types, membership types)
  - ✅ Admin type management UI with create/edit/delete/reorder
  - ✅ Removed is_system restriction - all types are now deletable
  - ✅ Squashed migrations into single initial schema
  - ✅ Generic default types in migrations (Member Meeting, Social, News, Awards, Member/Associate/Life Member)
  - ✅ Admin announcement editor with full CRUD
  - ✅ Announcement filtering by type and status (published/draft/featured/public)
  - ✅ Publish/unpublish workflow for announcements
  - ✅ Admin event management interface with full CRUD
  - ✅ Event filtering by type, visibility, and time (upcoming/past/all)
  - ✅ Sortable table columns for members and events
  - ✅ Seed data scaled to 100 members with faker
  - ✅ Admin member list with search/filter/pagination
  - ✅ Admin member detail page with full editing
  - ✅ Admin add new member page
  - ✅ Manual dues management (extend, set date, expire)
  - ✅ CSRF protection for all forms

## Quick Start Tasks

For getting a minimal viable product running:

1. Implement member repository with SQLite
2. Add basic authentication (no 2FA initially)
3. Create simple HTMX admin interface for member management
4. Add Stripe webhook handler for payment processing
5. Deploy to a VPS with SQLite and Caddy