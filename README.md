# zulip-messages-rs

Gather zulip messages from multiple instances in one place.

## Usage

First make a `config.json` file in the directory you want to run this program from.

You can get an api key from Settings > Your account > API key.
If you use github to login, you will need to add a password first.

```json
{
    "sites": [
        {
            "name": "rust-lang",
            "user": "info@example.com",
            "token": "abcdefghijklmnopqrstuvwxyz123456"
        },
    ]
}
```

Next you can run it using:

```bash
$ cargo run
```
