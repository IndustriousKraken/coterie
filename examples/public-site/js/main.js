/**
 * Main page JavaScript for HEHC public site
 */

// Store for full content to show in modals
const contentStore = {
    events: {},
    announcements: {}
};

document.addEventListener('DOMContentLoaded', () => {
    // Create modal element
    createModal();

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
 * Create the modal element
 */
function createModal() {
    const modal = document.createElement('div');
    modal.id = 'detail-modal';
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal">
            <div class="modal-header">
                <h3 id="modal-title"></h3>
                <button class="modal-close" onclick="closeModal()">&times;</button>
            </div>
            <div class="modal-body">
                <div id="modal-meta" class="modal-meta"></div>
                <div id="modal-content" class="modal-content"></div>
            </div>
        </div>
    `;
    document.body.appendChild(modal);

    // Close on overlay click
    modal.addEventListener('click', (e) => {
        if (e.target === modal) closeModal();
    });

    // Close on escape key
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') closeModal();
    });
}

/**
 * Show the modal with event details
 */
function showEventModal(eventId) {
    const event = contentStore.events[eventId];
    if (!event) return;

    const date = new Date(event.start_time);
    const dateStr = date.toLocaleDateString('en-US', {
        weekday: 'long',
        month: 'long',
        day: 'numeric',
        year: 'numeric'
    });
    const timeStr = date.toLocaleTimeString('en-US', {
        hour: 'numeric',
        minute: '2-digit'
    });

    document.getElementById('modal-title').textContent = event.title;
    document.getElementById('modal-meta').innerHTML = `
        <p><span class="label">Type:</span> <span class="value">${escapeHtml(event.event_type || 'Event')}</span></p>
        <p><span class="label">Date:</span> <span class="value">${dateStr}</span></p>
        <p><span class="label">Time:</span> <span class="value">${timeStr}</span></p>
        ${event.location ? `<p><span class="label">Location:</span> <span class="value">${escapeHtml(event.location)}</span></p>` : ''}
    `;
    document.getElementById('modal-content').textContent = event.description || 'No description available.';

    document.getElementById('detail-modal').classList.add('active');
    document.body.style.overflow = 'hidden';
}

/**
 * Show the modal with announcement details
 */
function showAnnouncementModal(announcementId) {
    const announcement = contentStore.announcements[announcementId];
    if (!announcement) return;

    const date = new Date(announcement.published_at || announcement.created_at);
    const dateStr = date.toLocaleDateString('en-US', {
        weekday: 'long',
        month: 'long',
        day: 'numeric',
        year: 'numeric'
    });

    document.getElementById('modal-title').textContent = announcement.title;
    document.getElementById('modal-meta').innerHTML = `
        <p><span class="label">Published:</span> <span class="value">${dateStr}</span></p>
    `;
    document.getElementById('modal-content').textContent = announcement.content || 'No content available.';

    document.getElementById('detail-modal').classList.add('active');
    document.body.style.overflow = 'hidden';
}

/**
 * Close the modal
 */
function closeModal() {
    document.getElementById('detail-modal').classList.remove('active');
    document.body.style.overflow = '';
}

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

        // Store events for modal access
        events.forEach(e => contentStore.events[e.id] = e);

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

        // Store events for modal access
        events.forEach(e => contentStore.events[e.id] = e);

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
 * Create HTML for an event card
 */
function createEventCard(event) {
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

    const maxLength = 150;
    const fullDescription = event.description || '';
    const needsTruncation = fullDescription.length > maxLength;
    const displayDescription = needsTruncation
        ? truncate(fullDescription, maxLength)
        : fullDescription;

    const readMoreLink = needsTruncation
        ? `<span class="read-more" onclick="showEventModal('${escapeHtml(event.id)}')">[more...]</span>`
        : '';

    return `
        <div class="event-card">
            <span class="event-type">${escapeHtml(event.event_type || 'event')}</span>
            <h3>${escapeHtml(event.title)}</h3>
            <p class="event-date">${dateStr} @ ${timeStr}</p>
            ${event.location ? `<p class="event-location">Location: ${escapeHtml(event.location)}</p>` : ''}
            ${displayDescription ? `<p class="event-description">${escapeHtml(displayDescription)}${readMoreLink}</p>` : ''}
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

        // Store announcements for modal access
        announcements.forEach(a => contentStore.announcements[a.id] = a);

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

    const maxLength = 200;
    const fullContent = announcement.content || '';
    const needsTruncation = fullContent.length > maxLength;
    const displayContent = needsTruncation
        ? truncate(fullContent, maxLength)
        : fullContent;

    const readMoreLink = needsTruncation
        ? `<span class="read-more" onclick="showAnnouncementModal('${escapeHtml(announcement.id)}')">[more...]</span>`
        : '';

    return `
        <div class="announcement">
            <h3>${escapeHtml(announcement.title)}</h3>
            <p class="announcement-date">${dateStr}</p>
            <p class="announcement-content">${escapeHtml(displayContent)}${readMoreLink}</p>
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
