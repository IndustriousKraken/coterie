## 1. Add error-path tests for `AnnouncementAdminService`

- [ ] 1.1 `update_errors_when_announcement_id_not_found` —
  call `svc.update(actor, Uuid::new_v4(), valid_input)` with a random
  UUID. Assert `Err(AppError::NotFound(msg))` where
  `msg.contains("Announcement not found")`. Assert that
  `audit_count(&pool, "update_announcement", &missing_id.to_string())`
  is `0`.
- [ ] 1.2 `delete_errors_when_announcement_id_not_found` —
  call `svc.delete(actor, Uuid::new_v4())` with a random UUID.
  Assert `Err(AppError::NotFound(msg))` where
  `msg.contains("Announcement not found")`. Assert no
  `delete_announcement` audit row was written for that id.
- [ ] 1.3 `publish_errors_when_announcement_id_not_found` —
  call `svc.publish(actor, Uuid::new_v4())` with a random UUID.
  Assert `Err(AppError::NotFound(msg))` where
  `msg.contains("Announcement not found")`. Assert no
  `publish_announcement` audit row was written for that id.
- [ ] 1.4 `unpublish_errors_when_announcement_id_not_found` —
  call `svc.unpublish(actor, Uuid::new_v4())` with a random UUID.
  Assert `Err(AppError::NotFound(msg))` where
  `msg.contains("Announcement not found")`. Assert no
  `unpublish_announcement` audit row was written for that id.
