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
- [ ] Member management interface
  - [ ] List/search members
  - [ ] Edit member details
  - [ ] Manual activation/expiration
- [ ] Event management interface
- [ ] Announcement editor
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

### Seed Data Improvements
- [x] Basic seed script with test accounts
- [ ] Realistic payment history (monthly dues over 1+ years per member)
- [ ] More calendar events (variety of types, past and upcoming)

## Notes

- **Current Status**: All core repositories implemented, authentication complete, ready for API handlers and testing
- **Next Step**: Create database seeding script, implement remaining API handlers, add integration tests
- **Blocking Issues**: None currently
- **Dependencies**: Need to evaluate specific Discord and Unifi API libraries
- **Completed Today**: 
  - ✅ SQLite Member Repository with full CRUD
  - ✅ SQLite Event Repository with attendance tracking
  - ✅ SQLite Announcement Repository with visibility control
  - ✅ SQLite Payment Repository with status management
  - ✅ Authentication system with sessions
  - ✅ Password hashing with Argon2id
  - ✅ Auth middleware (require_auth, require_admin)
  - ✅ Member management API handlers
  - ✅ Integration tests for member repository
  - ✅ All repositories now fully implemented and compiling
  - ✅ Database seeding script with test data

## Quick Start Tasks

For getting a minimal viable product running:

1. Implement member repository with SQLite
2. Add basic authentication (no 2FA initially)
3. Create simple HTMX admin interface for member management
4. Add Stripe webhook handler for payment processing
5. Deploy to a VPS with SQLite and Caddy