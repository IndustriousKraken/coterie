# announcement-markdown Specification

## Purpose
TBD - created by archiving change announcement-markdown-rendering. Update Purpose after archive.
## Requirements
### Requirement: Announcement Markdown is rendered by one shared sanitized pipeline

Announcement content and event descriptions SHALL be converted from Markdown to
HTML by a single shared server-side renderer that is the sole path from stored
Markdown to displayed HTML. The renderer SHALL keep raw/embedded HTML passthrough
disabled and SHALL restrict output to a fixed safe subset — paragraphs, line
breaks, emphasis, strong, strikethrough, links, lists, headings, inline code, code
blocks, and blockquotes — stripping every other element and attribute. Link URLs
SHALL be limited to the `http`, `https`, and `mailto` schemes; any other scheme
(for example `javascript:` or `data:`) SHALL be removed. Inline images from
Markdown SHALL NOT be emitted; a content type's dedicated image field is the only
image surface. The stored raw Markdown SHALL NOT be mutated by rendering.

Adding a content type to this pipeline SHALL NOT introduce a second renderer or a
second set of sanitization decisions. The safe subset, the scheme allow-list, and
the image exclusion are one set of rules with one implementation, so a change to
what is safe reaches every content type at once.

#### Scenario: Only the safe subset survives

- **WHEN** announcement Markdown contains a disallowed construct — raw
  `<script>`, an `<img>`, an `onclick` attribute, or a `javascript:`/`data:` URL
- **THEN** the rendered HTML SHALL NOT contain that element, attribute, or scheme

#### Scenario: Common formatting renders to safe HTML

- **WHEN** the Markdown contains bold, italic, strikethrough, a bulleted list,
  and an `https` link
- **THEN** each SHALL render as its safe HTML equivalent (for example `<strong>`,
  `<em>`, `<del>`, `<ul><li>`, and an `<a href>` with the preserved `https` URL)

#### Scenario: An event description is held to the same subset

- **WHEN** an event description contains raw `<script>`, an `<img>`, or a
  `javascript:` URL
- **THEN** the rendered HTML SHALL exclude it, identically to announcement content

### Requirement: Public announcement output carries server-rendered sanitized HTML

`GET /public/announcements` SHALL include, for each returned announcement, a
server-rendered sanitized `content_html` produced by the shared pipeline,
alongside the raw `content`, so a consumer can render formatted content without
running a Markdown parser or making a sanitization decision of its own. The
`GET /public/feed/rss` item description SHALL likewise carry the sanitized HTML
rendering of the announcement body. The added field SHALL be reflected in the
OpenAPI schema for the endpoint.

#### Scenario: content_html is present and sanitized

- **WHEN** a published public announcement with a Markdown body is fetched from
  `/public/announcements`
- **THEN** the response entry SHALL include a `content_html` field carrying the
  sanitized rendered HTML, and that HTML SHALL NOT contain a script element, an
  event-handler attribute, or an unsafe URL scheme

#### Scenario: RSS description carries rendered HTML

- **WHEN** `/public/feed/rss` is fetched
- **THEN** each item's description SHALL be the sanitized rendered HTML of the
  announcement body

### Requirement: Public event output carries server-rendered sanitized HTML

`GET /public/events` SHALL include, for each returned event, a server-rendered
sanitized rendering of the event description produced by the shared pipeline,
alongside the raw description, so a consumer can render formatted content without
running a Markdown parser or making a sanitization decision of its own. This
mirrors what the endpoint's announcement counterpart already provides. The added
field SHALL be reflected in the OpenAPI schema for the endpoint.

A members-only event's description is replaced by a fixed placeholder before it
leaves the API, and the rendered field SHALL be derived from the same sanitized
projection the raw field is, never from the underlying row. Rendering SHALL NOT
become a path by which a description that the projection withheld reaches a
public consumer.

Coterie's own event-facing surfaces SHALL render the description rather than
emitting it as text, so an organizer sees the same formatting the public does.

#### Scenario: The rendered description is present and sanitized

- **WHEN** a public event with a Markdown description is fetched from
  `/public/events`
- **THEN** the entry SHALL carry a rendered field holding the sanitized HTML, and
  that HTML SHALL NOT contain a script element, an event-handler attribute, or an
  unsafe URL scheme

#### Scenario: A members-only event renders only its placeholder

- **WHEN** a members-only event is projected into `/public/events`
- **THEN** the rendered field SHALL be derived from the sanitized placeholder and
  SHALL NOT contain any part of the event's real description

#### Scenario: Coterie's own pages render the description

- **WHEN** an event whose description contains Markdown emphasis is displayed on a
  Coterie-served event page
- **THEN** the emphasis SHALL render as formatting rather than as literal
  punctuation

