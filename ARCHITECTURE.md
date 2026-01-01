# Coterie Architecture

## Overview

Coterie uses a **dual-frontend architecture** to separate concerns between public-facing content and member/admin management functionality.

## System Components

### 1. Coterie Backend (API Server)
**Technology**: Rust with Axum framework  
**Purpose**: Central API server providing all business logic and data management

#### Public APIs (`/public/*`)
- No authentication required
- Designed for consumption by static sites
- Endpoints:
  - `POST /public/signup` - New member registration
  - `GET /public/events` - Public event listings (JSON/iCal)
  - `GET /public/announcements` - Public announcements (JSON)
  - `GET /public/feed/rss` - RSS feed of announcements
  - `GET /public/feed/calendar` - iCal calendar feed

#### Protected APIs (`/api/*`)
- Require authentication via session cookies
- Member and admin functionality
- Full CRUD operations on all resources

### 2. Public Website (Static Site)
**Technology**: Any static site generator (Hugo, Jekyll, Next.js, etc.)  
**Purpose**: Marketing, information, and public engagement

#### Features:
- **Informational Pages**: About, membership benefits, contact
- **Dynamic Content via API**:
  - Event calendar (fetched from `/public/events`)
  - News/announcements (fetched from `/public/announcements`)
  - Member signup form (posts to `/public/signup`)
  - Public member directory (if enabled)
- **Feed Integration**:
  - RSS feed for news readers
  - iCal subscription for calendars

#### Deployment:
- Can be hosted anywhere (GitHub Pages, Netlify, Vercel, S3)
- Fetches data client-side or at build time
- No server-side code required

### 3. Management Portal
**Technology**: HTMX + Alpine.js (server-side rendered)  
**Purpose**: Member self-service and admin management

#### Member Features:
- Login/logout
- Profile management
- Event RSVP
- Payment history
- Download receipts

#### Admin Features:
- Member management (approve, activate, expire)
- Event creation and management
- Announcement publishing
- Payment tracking
- Settings configuration
- Audit log viewing

#### Deployment:
- Served directly by Coterie backend
- Server-side rendered with HTMX for interactivity
- Minimal JavaScript (Alpine.js for UI enhancements)

## Data Flow Examples

### Public Event Display
```
Static Site → GET /public/events → Coterie API → JSON Response → Render on Page
```

### Member Signup
```
Static Site Form → POST /public/signup → Coterie API → Create Pending Member → Email Notification
```

### Admin Member Approval
```
Admin Portal → POST /api/members/:id/activate → Coterie API → Update Member → Integration Webhooks
```

## Integration Points

### For Static Sites
The public API is designed to be consumed by any static site generator:

```javascript
// Example: Fetching events for a static site
fetch('https://api.yourorg.com/public/events')
  .then(res => res.json())
  .then(events => {
    // Render events on your static site
  });
```

```html
<!-- Example: Embedding signup form -->
<form action="https://api.yourorg.com/public/signup" method="POST">
  <input name="email" type="email" required>
  <input name="username" required>
  <input name="full_name" required>
  <input name="password" type="password" required>
  <button type="submit">Join Us</button>
</form>
```

### For Calendar Apps
```
# Subscribe to events in any calendar app
https://api.yourorg.com/public/feed/calendar
```

### For RSS Readers
```
# Subscribe to announcements
https://api.yourorg.com/public/feed/rss
```

## Security Considerations

### Public API
- Rate limiting on all endpoints
- CORS configured for allowed domains
- No sensitive data exposed
- Signup requires email verification (pending implementation)

### Management Portal
- Session-based authentication
- Secure cookies (HttpOnly, SameSite, Secure)
- CSRF protection via token validation on state-changing requests
- Role-based access control (member vs admin)

### Static Site
- No secrets or API keys needed
- All API calls are to public endpoints
- Can use environment variables for API base URL

## Deployment Architecture

```
┌─────────────────┐      ┌──────────────────┐      ┌─────────────────┐
│   Static Site   │      │  Coterie Server  │      │    Database     │
│   (CDN/Edge)    │◄────►│    (VPS/Cloud)   │◄────►│    (SQLite)     │
└─────────────────┘      └──────────────────┘      └─────────────────┘
                                 │
                                 ▼
                         ┌──────────────────┐
                         │   Integrations   │
                         │ (Discord, Unifi) │
                         └──────────────────┘
```

## Configuration

### Static Site
Needs only the Coterie API base URL:
```javascript
const COTERIE_API = process.env.COTERIE_API_URL || 'https://api.yourorg.com';
```

### Coterie Server
Configured via environment variables and database settings:
- API keys (Stripe, Discord, etc.) - environment only
- Business logic (fees, text) - database settings via admin UI
- Server config (port, host) - environment variables

## Benefits of This Architecture

1. **Separation of Concerns**: Public content vs management functionality
2. **Independent Deployment**: Static site can be updated without touching the API
3. **Performance**: Static site can be CDN-cached globally
4. **Security**: Management portal behind authentication, public site has no secrets
5. **Flexibility**: Organizations can use any static site generator they prefer
6. **Cost-Effective**: Static hosting is free/cheap, only API server needs compute
7. **Developer Experience**: Frontend developers can work independently on the static site