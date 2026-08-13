# email-tokens Specification Delta

## MODIFIED Requirements

### Requirement: Tokens are 256 bits of cryptographic randomness

Plaintext tokens SHALL be generated from 32 random bytes (256 bits) drawn from a
cryptographically secure pseudorandom generator seeded by the operating system,
hex-encoded for inclusion in URLs.

The requirement is the property, not a particular function. Naming one crate's
call meant that an upstream rename — `rand::thread_rng()` becoming `rand::rng()`
between 0.9 and its predecessor — put the project in violation of canon without
anything about its security changing, and made the requirement unsatisfiable by
any version it could upgrade to. A specification that breaks when a dependency
renames a function is describing an implementation, not a requirement.

Every value the system generates as a secret SHALL be drawn from that same
generator: session and email tokens, TOTP shared secrets, recovery codes,
generated member passwords, content-security-policy nonces, and secrets produced
by the provisioning tool. A seedable, reproducible, or small-state generator
SHALL NOT be substituted at any of these sites.

That prohibition is stated because the failure it prevents is silent. Such a
substitution compiles, produces output of the right shape and length, and passes
any test that checks format — while making the values predictable. There is no
natural signal for it, so it needs one.

#### Scenario: Token value is unpredictable

- **WHEN** an attacker observes prior issued tokens
- **THEN** the next-issued token SHALL NOT be predictable; the keyspace is 2^256

#### Scenario: Secrets do not repeat across generations

- **WHEN** many values are generated at any of the secret-generating sites
- **THEN** no value SHALL repeat, and the sequence SHALL NOT be reproducible from
  a seed available to a caller

#### Scenario: A dependency rename does not violate the requirement

- **WHEN** the randomness library renames the call used to obtain the generator
- **THEN** the requirement SHALL remain satisfied so long as the generator is
  still a cryptographically secure, OS-seeded one
