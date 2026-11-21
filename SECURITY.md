# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's private reporting: the **Security** tab of the repository, then **Report a vulnerability**. Describe what you found, how to reproduce it and what an attacker gains. You will get an answer as soon as the maintainer can read it; this is a small project maintained in spare time, so please allow some days.

A good report gives the version (`dominet --version` or the image tag), the deployment (Docker Compose, which profiles), and the smallest steps that show the problem. Please do not test against other people's installations.

## Supported versions

Only the latest release receives fixes.

## What is in scope

The application, the Compose files and scripts of this repository, and the documentation when it recommends something unsafe. Problems in third-party images (PostgreSQL, Mosquitto, Grafana, ReductStore, MediaMTX, Zigbee2MQTT, nginx) belong to their own projects; tell us if our configuration makes them worse.

## Before you expose an installation

DomiNet is built for a home network. Read [docs/security.md](docs/security.md): which ports are open to the LAN, what has no password on purpose (camera viewing), and what must never be forwarded to the Internet. For remote access use the optional WireGuard profile or your own VPN.
