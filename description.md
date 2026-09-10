Create, personalize, and redeem Bitcoin Lightning gift cards from LNbits in a sandboxed WASM extension.

Gift Cards supports:

- Individual and bulk gift card creation
- Custom recipient, sender, message, expiry, and card designs
- Shareable redemption links and QR codes
- LNURL-withdraw redemption with compatible Lightning wallets
- Safe retry behavior when a redemption payment fails

Gift card state and wallet operations use the permission-controlled LNbits WASM host APIs, keeping the extension isolated from LNbits core.
