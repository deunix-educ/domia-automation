# Internal messaging

R5 lot 5, [ADR 0041](adr/0041-asynchronous-messaging.md) (model: [ADR 0037](adr/0037-android-relay-and-messaging.md), channels: [ADR 0038](adr/0038-delivery-channels.md), SMS: [ADR 0039](adr/0039-android-relay-server.md)).

An administrator writes a message to accounts; it goes out by email, by SMS (through a connected Android phone) or both.

## Pages

| Address | Who | Content |
|---|---|---|
| `/messaging/compose` | administrators | subject, message, channel (email / SMS / both), recipients: all active accounts, administrators, or chosen accounts |
| `/messaging` | administrators | the 100 latest sent messages |
| `/messaging/inbox` | any account | the 100 latest received messages, with your own email / SMS status |
| `/messaging/<id>` | sender, recipients, administrators | the message; the sender and administrators see the full **delivery table**, a recipient only their own status |

## How a message goes out

"Send" saves the message and opens its page right away; sending then runs in the background and **statuses change live**:

1. **Email**: one message per recipient who has an address (subject = message subject, plain text). SMTP server: `EMAIL_*` variables; sender: the `DEFAULT_FROM_EMAIL` setting.
2. **SMS**: handed to the **most recently seen online** Android device. The status stays "pending" until the app acknowledges; **with no acknowledgement after 60 s it turns into a failure**.
3. The message is "done" when nothing is pending any more. "Done" does not mean "everything succeeded": the table shows each failure with its reason (`email: …`, `sms: aucun appareil Android connecte`, `sms: pas d'accuse de l'appareil`…).

A recipient without an address (or without a mobile number) for the chosen channel is "not applicable" for that channel. The form flags these accounts and warns when no email server is configured or no phone is online.

## Rights and privacy

- Writing and listing sent messages: administrators only.
- A message that does not exist and a message that does not concern you get the same answer: "not found".
- Live follow: WebSocket topic `message:<id>`, same rights as the page.

## After a server stop

A message left "in progress" by a hard stop is closed at restart: its pending sends are marked failed ("interrompu par un redemarrage du serveur"). Nothing restarts by itself; the sender can send again.

## Notable behaviors

- **Asynchronous** sending (the request does not wait).
- SMS: **60 s delay**.
- Recipient without address or mobile: "not applicable".
- Lists of 100 rows instead of pages of 20; no drafts.
