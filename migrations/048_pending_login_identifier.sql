-- Carry the identifier the first factor was submitted with into the
-- second factor, so both spend the same credential budget.
--
-- The budget is keyed on the identifier AS SUBMITTED (rate-limiting
-- canon: keying it on the resolved account would make spending budget
-- an account-existence oracle). The web login accepts a username OR an
-- email, so a member who signs in with their username keys under that
-- username — but /login/totp only knows the member_id, and keying it on
-- the member's email gave those logins a second, fresh budget for the
-- 6-digit code space.
--
-- NULL for rows minted before this column existed; those callers fall
-- back to the member's email. Pending logins live 5 minutes, so the
-- NULLs are gone shortly after deploy.

ALTER TABLE pending_logins ADD COLUMN identifier TEXT;
