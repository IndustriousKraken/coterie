# Example Public Site

This is a sample public website demonstrating how to integrate with Coterie's public APIs. It showcases a fictional "Hipster Electronics Hacking Club" (HEHC) - a vintage computing enthusiast group.

## Features Demonstrated

- **Event Listing**: Fetches and displays upcoming events from `/public/events`
- **Announcements**: Shows recent announcements from `/public/announcements`
- **Signup Form**: Member registration via `POST /public/signup`
- **Calendar Integration**: Links to the iCal feed at `/public/feed/calendar`
- **RSS Feed**: Links to the RSS feed at `/public/feed/rss`

## Files

```
public-site/
├── index.html          # Landing page with events and announcements
├── events.html         # Full events listing with filtering
├── join.html           # Membership signup form
├── css/
│   └── style.css       # Retro terminal theme styling
├── js/
│   ├── api.js          # Coterie API client wrapper
│   └── main.js         # Page-specific JavaScript
└── README.md           # This file
```

## Configuration

Edit `js/api.js` to point to your Coterie instance:

```javascript
const COTERIE_API_URL = 'https://your-coterie-instance.com';
```

Or set it globally before loading the scripts:

```html
<script>
    window.COTERIE_API_URL = 'https://your-coterie-instance.com';
</script>
<script src="js/api.js"></script>
```

## Running Locally

This is a static site with no build step. You can serve it with any static file server:

```bash
# Python 3
cd examples/public-site
python -m http.server 3000

# Node.js (npx)
npx serve .

# PHP
php -S localhost:3000
```

Then visit `http://localhost:3000`

**Note**: For the API calls to work, you'll need a running Coterie instance. The default configuration points to `http://localhost:8080`.

## CORS Configuration

If your public site is hosted on a different domain than Coterie, you'll need to configure CORS on the Coterie backend. By default, Coterie allows requests from any origin for public endpoints.

## Customization

### Theming

The site uses CSS custom properties for easy theming. Edit the `:root` section in `style.css`:

```css
:root {
    --bg-dark: #0a0a0a;
    --text-primary: #33ff33;    /* Change for different terminal color */
    --text-amber: #ffb000;
    --accent: #00ffff;
    /* ... */
}
```

### Content

- Update the club name, tagline, and description in `index.html`
- Modify membership tiers and pricing in `join.html`
- Add your own event types and descriptions in `events.html`

## API Reference

See the main Coterie documentation for full API details. Key endpoints used:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/public/events` | GET | List upcoming public events |
| `/public/announcements` | GET | List public announcements |
| `/public/signup` | POST | Register a new member |
| `/public/feed/calendar` | GET | iCal feed of public events |
| `/public/feed/rss` | GET | RSS feed of announcements |

## Deployment

This static site can be hosted anywhere:

- **GitHub Pages**: Push to a `gh-pages` branch
- **Netlify/Vercel**: Connect your repo for auto-deploys
- **Traditional hosting**: Upload files via FTP/SFTP
- **Same server as Coterie**: Serve from `/var/www/` with your reverse proxy

### Example Nginx Configuration

```nginx
server {
    listen 80;
    server_name www.hehc.club;

    root /var/www/hehc-public;
    index index.html;

    location / {
        try_files $uri $uri/ =404;
    }
}
```

### Example Caddy Configuration

```
www.hehc.club {
    root * /var/www/hehc-public
    file_server
}
```
