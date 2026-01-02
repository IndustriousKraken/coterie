# Staging Deployment Setup

## On the Droplet (one-time)

### 1. Create system user
```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin coterie
```

### 2. Create deploy user
```bash
sudo useradd -m -s /bin/bash deploy
sudo usermod -aG coterie deploy
```

### 3. Create directories
```bash
sudo mkdir -p /opt/coterie/data
sudo chown -R coterie:coterie /opt/coterie
sudo chmod -R g+w /opt/coterie
```

### 4. Set up sudoers for deploy user
```bash
# Replace 'deploy' with your actual SSH user if different
echo "deploy ALL=(ALL) NOPASSWD: /bin/systemctl restart coterie, /bin/systemctl daemon-reload, /bin/cp /opt/coterie/deploy/coterie.service /etc/systemd/system/, /bin/chown -R coterie\:coterie /opt/coterie" | sudo tee /etc/sudoers.d/coterie
sudo chmod 440 /etc/sudoers.d/coterie
```

### 5. Add Caddy config
```bash
# Edit your Caddyfile (usually /etc/caddy/Caddyfile)
# Add the contents of Caddyfile.snippet, updating the domain

sudo systemctl reload caddy
```

### 6. DNS
Add an A record pointing your staging domain to the droplet IP.

## GitHub Secrets

Add these in your repo: Settings -> Secrets and variables -> Actions

| Secret | Description |
|--------|-------------|
| `SSH_PRIVATE_KEY` | Private key for SSH access to droplet |
| `REMOTE_HOST` | Droplet IP address or hostname |
| `REMOTE_USER` | SSH username (the deploy user) |

## First Deploy

After pushing to `staging`, the GitHub Action will:
1. Build the release binary
2. Rsync files to `/opt/coterie/`
3. Install the systemd service
4. Restart coterie

## Post-Deploy Configuration (one-time)

After the first successful deploy:

```bash
# 1. Copy and configure environment
sudo cp /opt/coterie/deploy/.env.staging /opt/coterie/.env

# 2. Generate a session secret
openssl rand -hex 32

# 3. Edit .env with your values
sudo nano /opt/coterie/.env
# - Replace CHANGE_ME_GENERATE_A_RANDOM_STRING with the generated secret
# - Update COTERIE_SERVER_BASE_URL to your domain (e.g., https://coterie.stage.grc.red)

# 4. Fix ownership
sudo chown -R coterie:coterie /opt/coterie

# 5. Enable and start the service
sudo systemctl enable coterie
sudo systemctl start coterie
```

## Seed Data (optional)

To populate with test data:
```bash
cd /opt/coterie
sudo systemctl stop coterie  # stop if running
sudo -u coterie ./seed
sudo systemctl start coterie
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
