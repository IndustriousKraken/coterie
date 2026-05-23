//! Dues management: `extend_dues` (add months) and `set_dues`
//! (set to a specific date). Both revive Expired→Active via the
//! repo's revival helper, audit, and dispatch `MemberUpdated`.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::{
    domain::Member,
    error::{AppError, Result},
};

use super::MemberService;

impl MemberService {
    /// Add `months` to the member's `dues_paid_until` (or to "now" if
    /// dues have already lapsed), revive Expired→Active, audit, and
    /// dispatch `MemberUpdated`. Validates `1..=120` — negative or
    /// absurd values would either wrap around as `u32` or dilute the
    /// audit log with junk entries.
    pub async fn extend_dues(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        months: i32,
    ) -> Result<Member> {
        use chrono::Months;

        if !(1..=120).contains(&months) {
            return Err(AppError::BadRequest(
                "Months must be between 1 and 120.".to_string(),
            ));
        }

        let old_member = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let now = Utc::now();
        let base_date = old_member
            .dues_paid_until
            .filter(|d| *d > now)
            .unwrap_or(now);

        let new_dues_date = base_date
            .checked_add_months(Months::new(months as u32))
            .unwrap_or(base_date);

        self.member_repo
            .set_dues_paid_until_with_revival(member_id, new_dues_date)
            .await?;

        self.audit_service
            .log(
                Some(actor_id),
                "extend_dues",
                "member",
                &member_id.to_string(),
                None,
                Some(&format!(
                    "+{} months → {}",
                    months,
                    new_dues_date.format("%Y-%m-%d")
                )),
                None,
            )
            .await;

        self.dispatch_member_updated(member_id, old_member).await
    }

    /// Set the member's `dues_paid_until` to the end of `naive_date`
    /// (23:59:59 UTC). Same revival/audit/dispatch chain as
    /// `extend_dues`, but sets rather than adds.
    pub async fn set_dues(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        naive_date: NaiveDate,
    ) -> Result<Member> {
        let dues_date: DateTime<Utc> = naive_date.and_hms_opt(23, 59, 59).unwrap().and_utc();

        let old_member = self
            .member_repo
            .find_by_id(member_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        self.member_repo
            .set_dues_paid_until_with_revival(member_id, dues_date)
            .await?;

        self.audit_service
            .log(
                Some(actor_id),
                "set_dues",
                "member",
                &member_id.to_string(),
                None,
                Some(&dues_date.format("%Y-%m-%d").to_string()),
                None,
            )
            .await;

        self.dispatch_member_updated(member_id, old_member).await
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::error::AppError;
    use chrono::NaiveDate;

    #[tokio::test]
    async fn extend_dues_validates_range() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let bad = svc.extend_dues(actor.id, target.id, 0).await;
        assert!(matches!(bad, Err(AppError::BadRequest(_))));
        let bad_high = svc.extend_dues(actor.id, target.id, 121).await;
        assert!(matches!(bad_high, Err(AppError::BadRequest(_))));

        let ok = svc.extend_dues(actor.id, target.id, 12).await.unwrap();
        assert!(ok.dues_paid_until.is_some());
        assert_eq!(audit_count(&pool, "extend_dues", &target.id).await, 1);
    }

    #[tokio::test]
    async fn set_dues_writes_audit() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_member(&pool, "admin@example.com", "admin").await;
        let target = make_member(&pool, "tgt@example.com", "target").await;

        let date = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        let result = svc.set_dues(actor.id, target.id, date).await.unwrap();

        let dpu = result.dues_paid_until.unwrap();
        assert_eq!(dpu.format("%Y-%m-%d").to_string(), "2027-01-01");
        assert_eq!(audit_count(&pool, "set_dues", &target.id).await, 1);
    }
}
