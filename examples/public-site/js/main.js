/**
 * Main page JavaScript for HEHC public site
 */

document.addEventListener('DOMContentLoaded', () => {
    // Load upcoming events on homepage
    if (document.getElementById('events-list')) {
        loadUpcomingEvents();
    }

    // Load announcements on homepage
    if (document.getElementById('announcements-list')) {
        loadAnnouncements();
    }

    // Handle signup form
    const signupForm = document.getElementById('signup-form');
    if (signupForm) {
        signupForm.addEventListener('submit', handleSignup);
    }

    // Load all events on events page
    if (document.getElementById('all-events-list')) {
        loadAllEvents();
    }

    // Update typing animation text after delay
    setTimeout(() => {
        const typingEl = document.querySelector('.typing');
        if (typingEl) {
            typingEl.textContent = 'System ready. Welcome, human.';
        }
    }, 2000);
});

/**
 * Load and display upcoming events
 */
async function loadUpcomingEvents() {
    const container = document.getElementById('events-list');

    try {
        const events = await CoterieAPI.getEvents({ limit: 3 });

        if (events.length === 0) {
            container.innerHTML = '<p class="no-data">No upcoming events scheduled. Check back soon!</p>';
            return;
        }

        container.innerHTML = events.map(event => createEventCard(event)).join('');
    } catch (error) {
        container.innerHTML = `
            <div class="error">
                <p>ERROR: Unable to connect to event database.</p>
                <p class="error-detail">${error.message}</p>
            </div>
        `;
    }
}

/**
 * Load all events for the events page
 */
async function loadAllEvents() {
    const container = document.getElementById('all-events-list');

    try {
        const events = await CoterieAPI.getEvents({ limit: 50 });

        if (events.length === 0) {
            container.innerHTML = '<p class="no-data">No upcoming events scheduled. Check back soon!</p>';
            return;
        }

        container.innerHTML = events.map(event => createEventCard(event, true)).join('');
    } catch (error) {
        container.innerHTML = `
            <div class="error">
                <p>ERROR: Unable to connect to event database.</p>
                <p class="error-detail">${error.message}</p>
            </div>
        `;
    }
}

/**
 * Create HTML for an event card
 */
function createEventCard(event, showFull = false) {
    const date = new Date(event.start_time);
    const dateStr = date.toLocaleDateString('en-US', {
        weekday: 'short',
        month: 'short',
        day: 'numeric'
    });
    const timeStr = date.toLocaleTimeString('en-US', {
        hour: 'numeric',
        minute: '2-digit'
    });

    const description = showFull
        ? event.description || ''
        : truncate(event.description || '', 100);

    return `
        <div class="event-card">
            <span class="event-type">${escapeHtml(event.event_type || 'event')}</span>
            <h3>${escapeHtml(event.title)}</h3>
            <p class="event-date">${dateStr} @ ${timeStr}</p>
            ${event.location ? `<p class="event-location">Location: ${escapeHtml(event.location)}</p>` : ''}
            ${description ? `<p class="event-description">${escapeHtml(description)}</p>` : ''}
        </div>
    `;
}

/**
 * Load and display recent announcements
 */
async function loadAnnouncements() {
    const container = document.getElementById('announcements-list');

    try {
        const announcements = await CoterieAPI.getAnnouncements({ limit: 3 });

        if (announcements.length === 0) {
            container.innerHTML = '<p class="no-data">No recent announcements.</p>';
            return;
        }

        container.innerHTML = announcements.map(ann => createAnnouncementCard(ann)).join('');
    } catch (error) {
        container.innerHTML = `
            <div class="error">
                <p>ERROR: Unable to fetch announcements.</p>
                <p class="error-detail">${error.message}</p>
            </div>
        `;
    }
}

/**
 * Create HTML for an announcement card
 */
function createAnnouncementCard(announcement) {
    const date = new Date(announcement.published_at || announcement.created_at);
    const dateStr = date.toLocaleDateString('en-US', {
        month: 'short',
        day: 'numeric',
        year: 'numeric'
    });

    return `
        <div class="announcement">
            <h3>${escapeHtml(announcement.title)}</h3>
            <p class="announcement-date">${dateStr}</p>
            <p class="announcement-content">${escapeHtml(truncate(announcement.content || '', 200))}</p>
        </div>
    `;
}

/**
 * Handle signup form submission
 */
async function handleSignup(e) {
    e.preventDefault();

    const form = e.target;
    const submitBtn = form.querySelector('button[type="submit"]');
    const messageEl = document.getElementById('signup-message');

    // Gather form data
    const data = {
        email: form.email.value,
        username: form.username.value,
        full_name: form.full_name.value,
        password: form.password.value,
        membership_type: form.membership_type.value
    };

    // Validate password match
    if (form.password.value !== form.password_confirm.value) {
        showMessage(messageEl, 'ERROR: Passwords do not match.', 'error');
        return;
    }

    // Disable form during submission
    submitBtn.disabled = true;
    submitBtn.textContent = 'Processing...';
    messageEl.innerHTML = '';

    try {
        const result = await CoterieAPI.signup(data);
        showMessage(messageEl, 'SUCCESS: Registration complete! Check your email to verify your account.', 'success');
        form.reset();
    } catch (error) {
        showMessage(messageEl, `ERROR: ${error.message}`, 'error');
    } finally {
        submitBtn.disabled = false;
        submitBtn.textContent = 'Submit Application';
    }
}

/**
 * Show a message to the user
 */
function showMessage(element, message, type = 'info') {
    element.className = `message message-${type}`;
    element.innerHTML = `<p>${escapeHtml(message)}</p>`;
}

/**
 * Truncate text to a maximum length
 */
function truncate(text, maxLength) {
    if (text.length <= maxLength) return text;
    return text.substring(0, maxLength).trim() + '...';
}

/**
 * Escape HTML to prevent XSS
 */
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}
