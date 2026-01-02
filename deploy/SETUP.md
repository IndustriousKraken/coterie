# Staging Deployment Setup

## On the Droplet (one-time)

### 1. Create system user
```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin coterie
```

### 2. Create directories
```bash
sudo mkdir -p /opt/coterie/data
sudo chown -R coterie:coterie /opt/coterie
```

### 3. Configure environment
```bash
# Copy and edit the config files
sudo cp /opt/coterie/deploy/.env.staging /opt/coterie/.env
sudo cp /opt/coterie/deploy/config.toml.staging /opt/coterie/config.toml

# Generate a session secret
openssl rand -hex 32

# Edit .env and config.toml with your values
sudo nano /opt/coterie/.env
sudo nano /opt/coterie/config.toml
```

### 4. Add Caddy config
```bash
# Edit your Caddyfile (usually /etc/caddy/Caddyfile)
# Add the contents of Caddyfile.snippet, updating the domain

sudo systemctl reload caddy
```

### 5. Set up sudoers for deploy user
```bash
# Replace 'deployuser' with your actual SSH user
echo "deployuser ALL=(ALL) NOPASSWD: /bin/systemctl restart coterie, /bin/systemctl daemon-reload, /bin/cp /opt/coterie/deploy/coterie.service /etc/systemd/system/, /bin/chown -R coterie\:coterie /opt/coterie" | sudo tee /etc/sudoers.d/coterie
sudo chmod 440 /etc/sudoers.d/coterie
```

### 6. DNS
Add an A record pointing your staging domain to the droplet IP.

## GitHub Secrets

Add these in your repo: Settings → Secrets and variables → Actions

| Secret | Description |
|--------|-------------|
| `SSH_PRIVATE_KEY` | Private key for SSH access to droplet |
| `REMOTE_HOST` | Droplet IP address or hostname |
| `REMOTE_USER` | SSH username (the deploy user) |

## First Deploy

After pushing to `main`, the GitHub Action will:
1. Build the release binary
2. Rsync files to `/opt/coterie/`
3. Install the systemd service
4. Restart coterie

## Seed Data (optional)

To populate with test data:
```bash
cd /opt/coterie
sudo -u coterie ./coterie seed  # if you have a seed command
# OR copy a pre-seeded database
```

## Useful Commands

```bash
# Check status
sudo systemctl status coterie

# View logs
sudo journalctl -u coterie -f

# Restart manually
sudo systemctl restart coterie

# Check Caddy logs
sudo journalctl -u caddy -f
```
