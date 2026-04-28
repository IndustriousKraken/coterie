# Deploying Coterie on AWS

End-to-end walkthrough for running Coterie on AWS with EC2 + EBS for
the database and S3 for offsite backups.

This guide parallels `DEPLOY-DIGITALOCEAN.md` step-for-step. If you've
read that one, the AWS one mostly differs in the provisioning commands
and the resource names; the post-install steps are identical.

Target audience: an operator who has never deployed Coterie before.
Time to first running instance: about 60 minutes (AWS has more dials).

---

## Choosing between EC2 and Lightsail

AWS has two reasonable paths:

| | **Lightsail** | **EC2 + EBS** |
| --- | --- | --- |
| Pricing | flat per-month, predictable | per-hour + per-GB-month for EBS |
| Networking | included static IP, simple firewall | full VPC + security groups |
| Storage | bundled (resize the instance) | separate EBS volumes (decoupled lifecycle) |
| Backups | built-in instance snapshots | EBS snapshots + S3 + AMIs |
| Best when | small org, predictable workload | future flexibility, existing AWS footprint |

**Pick Lightsail** if you're new to AWS and just want something that
works. **Pick EC2 + EBS** if you already have an AWS footprint or you
want the database on a volume independent from the compute.

The instructions below cover **EC2 + EBS** (the more general path).
A short Lightsail variant is at the bottom.

---

## What you'll have at the end (EC2 + EBS path)

```
                       coterie.example.com
                                │
                          Route 53 / ext DNS
                                │
                                ▼
                       ┌────────────────────┐
                       │  EC2 (t4g.small)   │
                       │  ┌──────────────┐  │
                       │  │   Caddy      │  │   (TLS, security group: 80/443/22)
                       │  │   :443       │  │
                       │  └──────┬───────┘  │
                       │         │          │
                       │  ┌──────▼───────┐  │
                       │  │   Coterie    │  │   (systemd, :8080)
                       │  └──────┬───────┘  │
                       └─────────┼──────────┘
                                 │
                       ┌─────────▼──────────┐
                       │  EBS gp3, 10 GiB   │   /var/lib/coterie
                       │  (separate from    │   (snapshots run nightly)
                       │   root volume)     │
                       └────────────────────┘
                                 │
                                 ▼ daily 03:30
                       ┌────────────────────┐
                       │  S3                │   offsite backups
                       │  (with lifecycle:  │   IA after 30d, Glacier after 90d
                       │   tier to IA/Glacier)│
                       └────────────────────┘
```

---

## 0. Prerequisites

- An AWS account with an IAM user that has `EC2`, `EBS`, `S3`, and
  `IAM` permissions
- The `aws` CLI configured (`aws configure`)
- A domain (Route 53 or any external registrar)
- A built Coterie binary, OR willingness to build on the EC2 instance

---

## 1. Sizing

Coterie is small. ARM (Graviton) is fine and ~20% cheaper than x86:

| Org size              | Instance type     | Cost/month (Apr 2026) |
| --------------------- | ----------------- | --------------------- |
| < 200 members         | t4g.micro         | ~$6                   |
| 200–2000 members      | t4g.small         | ~$12                  |
| > 2000 members        | t4g.medium        | ~$24                  |

If you build on the host, allow ~2 GB free for the cargo build
cache. `t4g.small` builds in ~10 minutes.

---

## 2. Provision the EC2 instance + EBS volume

```bash
REGION=us-east-1
KEY_NAME=coterie-prod              # an existing key pair name
AZ=${REGION}a

# Find the latest Ubuntu 24.04 ARM64 AMI
AMI=$(aws ec2 describe-images \
    --owners 099720109477 \
    --filters \
        "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" \
        "Name=state,Values=available" \
    --query 'Images | sort_by(@, &CreationDate) | [-1].ImageId' \
    --output text \
    --region "$REGION")

# Create a security group that allows 22, 80, 443
SG_ID=$(aws ec2 create-security-group \
    --group-name coterie-prod \
    --description "Coterie production" \
    --region "$REGION" \
    --query GroupId --output text)

aws ec2 authorize-security-group-ingress --group-id "$SG_ID" \
    --protocol tcp --port 22 --cidr 0.0.0.0/0 --region "$REGION"
aws ec2 authorize-security-group-ingress --group-id "$SG_ID" \
    --protocol tcp --port 80 --cidr 0.0.0.0/0 --region "$REGION"
aws ec2 authorize-security-group-ingress --group-id "$SG_ID" \
    --protocol tcp --port 443 --cidr 0.0.0.0/0 --region "$REGION"

# Note: lock down :22 to your home IP for tighter posture:
# --cidr $(curl -s https://api.ipify.org)/32

# Launch instance with an extra 10 GiB gp3 EBS volume mounted as /dev/sdh
INSTANCE_ID=$(aws ec2 run-instances \
    --image-id "$AMI" \
    --instance-type t4g.small \
    --key-name "$KEY_NAME" \
    --security-group-ids "$SG_ID" \
    --placement AvailabilityZone="$AZ" \
    --block-device-mappings '[
        {"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":12,"VolumeType":"gp3"}},
        {"DeviceName":"/dev/sdh","Ebs":{"VolumeSize":10,"VolumeType":"gp3","DeleteOnTermination":false}}
    ]' \
    --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=coterie-prod}]' \
    --region "$REGION" \
    --query 'Instances[0].InstanceId' \
    --output text)

echo "Instance: $INSTANCE_ID"

# Wait for it to start, then get the public IP
aws ec2 wait instance-running --instance-ids "$INSTANCE_ID" --region "$REGION"
PUBLIC_IP=$(aws ec2 describe-instances \
    --instance-ids "$INSTANCE_ID" \
    --query 'Reservations[0].Instances[0].PublicIpAddress' \
    --output text --region "$REGION")
echo "Public IP: $PUBLIC_IP"
```

**`DeleteOnTermination: false`** on the data volume is important —
if you accidentally terminate the instance, the volume (and the DB)
survives.

---

## 3. SSH in, mount the volume

```bash
ssh ubuntu@$PUBLIC_IP

sudo -i

# Find the volume. Modern AMIs use NVMe naming; the AWS-provided
# `ec2-describe-disk` is helpful but `lsblk -o NAME,SIZE,FSTYPE,MOUNTPOINT`
# is universal:
lsblk
# nvme0n1   12G              /
# nvme1n1   10G              <— this is /dev/sdh

# Format (first time only)
mkfs.ext4 /dev/nvme1n1

# Mount target
mkdir -p /var/lib/coterie

# Persistent mount via /etc/fstab. AWS recommends nofail + UUID to
# avoid boot hangs if the volume detaches.
UUID=$(blkid -s UUID -o value /dev/nvme1n1)
echo "UUID=$UUID /var/lib/coterie ext4 defaults,nofail,discard 0 2" >> /etc/fstab
mount -a

df -h /var/lib/coterie
```

---

## 4. System packages

```bash
apt-get update
apt-get install -y --no-install-recommends \
    sqlite3 ca-certificates curl \
    debian-keyring debian-archive-keyring apt-transport-https

# Caddy
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | tee /etc/apt/sources.list.d/caddy-stable.list
apt-get update
apt-get install -y caddy

# AWS CLI for backups (S3 push)
apt-get install -y awscli
```

---

## 5. Deploy the Coterie code

Same options as DigitalOcean — rsync from your dev machine, or build
on the host.

```bash
# From your local machine:
rsync -avz target/release/coterie static/ deploy/ .env.example \
    ubuntu@$PUBLIC_IP:/tmp/coterie-deploy/

# Then on the host:
ssh ubuntu@$PUBLIC_IP
sudo mkdir -p /opt/coterie
sudo cp -r /tmp/coterie-deploy/* /opt/coterie/
sudo bash /opt/coterie/deploy/install.sh
```

Or build on the host (slower, but no architecture mismatch concerns —
your laptop is probably x86, the t4g instance is ARM):

```bash
# On the host:
sudo apt-get install -y build-essential pkg-config libssl-dev git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source $HOME/.cargo/env
git clone https://github.com/your-org/coterie /tmp/coterie-build
cd /tmp/coterie-build
make release
sudo mkdir -p /opt/coterie
sudo cp target/release/coterie /opt/coterie/
sudo cp -r static deploy /opt/coterie/
sudo cp .env.example /opt/coterie/
sudo bash /opt/coterie/deploy/install.sh
```

---

## 6. Configure `.env`

Same as DigitalOcean step 7 — copy `.env.example`, generate a session
secret with `openssl rand -hex 32`, set `BASE_URL` to your https URL,
set `DATA_DIR` to `/var/lib/coterie`. See `DEPLOY-DIGITALOCEAN.md`
section 7 for the field-by-field walkthrough.

---

## 7. Configure Caddy

Identical to the DO guide:

```bash
sudo cp /opt/coterie/deploy/Caddyfile.example /etc/caddy/Caddyfile
sudo nano /etc/caddy/Caddyfile     # update domain
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

---

## 8. DNS

Point `coterie.example.com` at the EC2 public IP.

If you're using Route 53:

```bash
ZONE_ID=Z1234567890ABC               # your hosted zone
aws route53 change-resource-record-sets --hosted-zone-id "$ZONE_ID" \
    --change-batch "{
      \"Changes\":[{
        \"Action\":\"UPSERT\",
        \"ResourceRecordSet\":{
          \"Name\":\"coterie.example.com.\",
          \"Type\":\"A\",
          \"TTL\":300,
          \"ResourceRecords\":[{\"Value\":\"$PUBLIC_IP\"}]
        }
      }]
    }"
```

For a long-lived deployment, attach an **Elastic IP** to the instance
so the address survives stop/start. ~$0/month while attached, charged
only when reserved-but-unattached:

```bash
EIP_ALLOC=$(aws ec2 allocate-address --region "$REGION" --query AllocationId --output text)
aws ec2 associate-address --instance-id "$INSTANCE_ID" \
    --allocation-id "$EIP_ALLOC" --region "$REGION"
```

---

## 9. Start Coterie

```bash
sudo systemctl enable --now coterie
sudo journalctl -u coterie -f
```

Visit `https://coterie.example.com` — setup page appears, create your
first admin.

---

## 10. Schedule backups (S3)

Create the bucket and a dedicated IAM user with write-only access:

```bash
BUCKET=my-coterie-backups-prod
aws s3 mb "s3://$BUCKET" --region "$REGION"

# Block public access (defense in depth — Coterie backups should never
# be public regardless of policy)
aws s3api put-public-access-block --bucket "$BUCKET" \
    --public-access-block-configuration \
    "BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true"

# Lifecycle: tier to IA after 30 days, Glacier after 90, expire at 365
cat > /tmp/lifecycle.json <<EOF
{
    "Rules": [{
        "ID": "tier-and-expire",
        "Status": "Enabled",
        "Filter": {"Prefix": "prod/"},
        "Transitions": [
            {"Days": 30, "StorageClass": "STANDARD_IA"},
            {"Days": 90, "StorageClass": "GLACIER"}
        ],
        "Expiration": {"Days": 365}
    }]
}
EOF
aws s3api put-bucket-lifecycle-configuration \
    --bucket "$BUCKET" --lifecycle-configuration file:///tmp/lifecycle.json

# Create an IAM user that can write but not read or delete (write-only
# defense: a compromised host can't read your old backups or wipe the
# bucket)
aws iam create-user --user-name coterie-backup
aws iam put-user-policy --user-name coterie-backup --policy-name coterie-backup-write \
    --policy-document "{
      \"Version\":\"2012-10-17\",
      \"Statement\":[{
        \"Effect\":\"Allow\",
        \"Action\":[\"s3:PutObject\"],
        \"Resource\":\"arn:aws:s3:::$BUCKET/prod/*\"
      }]
    }"

aws iam create-access-key --user-name coterie-backup
# Save AccessKeyId + SecretAccessKey from the output
```

Then on the EC2 host:

```bash
sudo tee /etc/default/coterie-backup > /dev/null <<EOF
COTERIE_BACKUP_S3_URI=s3://my-coterie-backups-prod/prod/
AWS_ACCESS_KEY_ID=<from create-access-key>
AWS_SECRET_ACCESS_KEY=<from create-access-key>
AWS_DEFAULT_REGION=us-east-1
EOF
sudo chmod 0600 /etc/default/coterie-backup

sudo cp /opt/coterie/deploy/coterie-backup.service /etc/systemd/system/
sudo cp /opt/coterie/deploy/coterie-backup.timer   /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now coterie-backup.timer

# Trigger one immediately to verify
sudo systemctl start coterie-backup.service
sudo journalctl -u coterie-backup -n 50
```

---

## 11. Snapshot strategy

Two layers in addition to the daily SQLite backups:

1. **EBS snapshots** of the data volume. AWS Backup or DLM can run
   these on a schedule. Recommended: nightly, retain 7. ~$0.05/GB/mo
   (incremental).
   ```bash
   aws ec2 create-snapshot --volume-id <vol-id> \
       --description "coterie-data manual" --region "$REGION"
   ```
2. **Bucket versioning** on the S3 backup bucket — the lifecycle
   config above doesn't cover overwrites. Enable if your retention
   needs versioned history:
   ```bash
   aws s3api put-bucket-versioning --bucket "$BUCKET" \
       --versioning-configuration Status=Enabled
   ```

The Coterie-internal daily backup is the primary recovery vehicle.
EBS snapshots are belt-and-suspenders for the volume itself; bucket
versioning is for the backup files.

---

## Lightsail variant (alternative to steps 1–3)

If you'd rather skip the VPC/EBS dance:

```bash
# Create a Lightsail instance with bundled storage
aws lightsail create-instances \
    --instance-names coterie-prod \
    --availability-zone us-east-1a \
    --blueprint-id ubuntu_24_04 \
    --bundle-id small_3_0 \
    --region us-east-1
```

Lightsail attaches a static IP via the console (or `attach-static-ip`).
The data volume is part of the instance — no separate mount step. All
subsequent steps (Caddy, install.sh, Coterie config, DNS, S3 backups)
are identical.

The downside: when you outgrow Lightsail or want EBS-style snapshot
flexibility, you have to migrate. The migration is a standard restore
into a fresh EC2 instance — see `MIGRATION.md`.

---

## Troubleshooting

**`unable to connect` to instance after launch** — security group
might not have port 22 open from your IP. Check `aws ec2 describe-
security-groups --group-ids $SG_ID`.

**Caddy fails to provision a cert** — most common cause is DNS not
yet pointing at the instance, or Route 53's TTL hasn't propagated.
Run `dig coterie.example.com @8.8.8.8` from a non-AWS machine.

**`502 Bad Gateway`** — Caddy is up but Coterie is down. Check
`journalctl -u coterie -n 100`. Most common: `.env` typo (the
`COTERIE__` prefix uses double underscores between sections).

**Backup fails with `NoCredentialProviders`** — `/etc/default/coterie-
backup` isn't being read. Verify the systemd unit's `EnvironmentFile=-
/etc/default/coterie-backup` line and that the file's mode is 0600
(too restrictive can also cause issues; 0600 owned by root is correct).

**EBS volume detached after instance reboot** — check `/etc/fstab` is
using a UUID, not `/dev/nvme1n1` directly. NVMe device order is not
guaranteed across boots.
