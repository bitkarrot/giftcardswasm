// Gift Cards Redeem Page — Vue 3 + Quasar, CSP-safe
// Uses bridge (postMessage) instead of fetch (blocked by connect-src 'none')
(function () {
  'use strict';

  var API_BASE = '/api/v1/ext/giftcardswasm';
  var IMG_BASE = '/ext-assets/giftcardswasm/image';

  // Sample template dimensions (must match index.js)
  var SAMPLE_TEMPLATES = [
    {value: 'GiftBoxes',        label: 'Gift Boxes',         w: 825, h: 638},
    {value: 'GiftCard',         label: 'Gift Card',          w: 825, h: 638},
    {value: 'HappyBirthday',    label: 'Happy Birthday',     w: 825, h: 638},
    {value: 'MerryXmas',        label: 'Merry Xmas',         w: 825, h: 638},
    {value: 'OrangeCard',       label: 'Orange Card',        w: 825, h: 638},
    {value: 'PurpleGift',       label: 'Purple Gift',        w: 825, h: 638},
    {value: 'SatsGiftCard',     label: 'Sats Gift Card',     w: 825, h: 638},
    {value: 'SeasonsGreetings', label: "Season's Greetings", w: 825, h: 638}
  ];

  async function apiCall(method, path, body) {
    await window.LNbitsBridge.connect();
    var result = await window.LNbitsBridge.callApi(method, API_BASE + path, body);
    if (result && typeof result === 'object' && 'ok' in result && 'data' in result) {
      return result.data;
    }
    return result;
  }

  // --- Minimal bech32 encoder (for LNURL bech32 encoding) ---
  var BECH32_CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';

  function bech32Polymod(values) {
    var chk = 1;
    var GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    for (var i = 0; i < values.length; i++) {
      var top = chk >> 25;
      chk = ((chk & 0x1ffffff) << 5) ^ values[i];
      for (var j = 0; j < 5; j++) {
        if ((top >> j) & 1) {
          chk ^= GEN[j];
        }
      }
    }
    return chk;
  }

  function bech32HrpExpand(hrp) {
    var ret = [];
    var i;
    for (i = 0; i < hrp.length; i++) {
      ret.push(hrp.charCodeAt(i) >> 5);
    }
    ret.push(0);
    for (i = 0; i < hrp.length; i++) {
      ret.push(hrp.charCodeAt(i) & 31);
    }
    return ret;
  }

  function bech32CreateChecksum(hrp, data) {
    var values = bech32HrpExpand(hrp).concat(data).concat([0, 0, 0, 0, 0, 0]);
    var mod = bech32Polymod(values) ^ 1;
    var ret = [];
    for (var i = 0; i < 6; i++) {
      ret.push((mod >> (5 * (5 - i))) & 31);
    }
    return ret;
  }

  function convertBits(bytes, fromBits, toBits, pad) {
    var acc = 0, bits = 0;
    var ret = [];
    var maxv = (1 << toBits) - 1;
    var maxAcc = (1 << (fromBits + toBits - 1)) - 1;
    for (var i = 0; i < bytes.length; i++) {
      acc = ((acc << fromBits) | bytes[i]) & maxAcc;
      bits += fromBits;
      while (bits >= toBits) {
        bits -= toBits;
        ret.push((acc >> bits) & maxv);
      }
    }
    if (pad && bits) {
      ret.push((acc << (toBits - bits)) & maxv);
    }
    return ret;
  }

  function bech32Encode(hrp, bytes) {
    var data = convertBits(bytes, 8, 5, true);
    var checksum = bech32CreateChecksum(hrp, data);
    var combined = data.concat(checksum);
    var ret = hrp + '1';
    for (var i = 0; i < combined.length; i++) {
      ret += BECH32_CHARSET[combined[i]];
    }
    return ret;
  }

  // --- Vue app ---
  var app = Vue.createApp({
    render: window.REDEEM_RENDER_FN(),
    data: function () {
      return {
        giftCard: null,
        loading: true,
        tokenHash: null,
        error: false,
        copied: false,
        tab: 'bech32',
        nfcTagWriting: false,
        nfcSupported: typeof NDEFReader !== 'undefined',
        imageDialogShow: false,
        imageDialogUrl: '',
        imageDialogTitle: 'Gift Card Image',
        imageDialogInstructions: ''
      };
    },
    computed: {
      lnurlUrl: function () {
        if (!this.tokenHash) return '';
        var baseUrl = window.location.origin;
        return baseUrl + '/api/v1/ext/giftcardswasm/lnurl/' + this.tokenHash;
      },
      lnurl: function () {
        if (!this.lnurlUrl) return '';
        if (this.tab === 'bech32') {
          var bytes = new TextEncoder().encode(this.lnurlUrl);
          var bech32 = bech32Encode('lnurl', Array.from(bytes));
          return 'lightning:' + bech32.toUpperCase();
        }
        // lud17: swap https:// -> lnurlw://
        return this.lnurlUrl.replace('https://', 'lnurlw://');
      },
      lnurlString: function () {
        if (!this.lnurl) return '';
        return this.lnurl.replace(/^(lightning|lnurlw):/i, '');
      }
    },
    watch: {
      tab: function () {
        var self = this;
        this.$nextTick(function () {
          self.renderQR();
        });
      }
    },
    mounted: function () {
      var self = this;
      // Auto-detect dark mode from system preference
      if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
        document.body.classList.add('body--dark');
        if (this.$q && this.$q.dark) this.$q.dark.set(true);
      }
      this.loadGiftCard().then(function () {
        var params = new URLSearchParams(window.location.search);
        if (params.get('error') === '1') {
          self.error = true;
        }
        self.$nextTick(function () {
          self.renderQR();
        });
      });
    },
    methods: {
      clearError: function () {
        this.error = false;
      },

      async copyLnurl() {
        if (!this.lnurlString) return;
        try {
          await navigator.clipboard.writeText(this.lnurlString);
          this.copied = true;
          var self = this;
          setTimeout(function () { self.copied = false; }, 2000);
          this.$q.notify({ type: 'positive', message: 'LNURL copied to clipboard!' });
        } catch (e) {
          console.error('Failed to copy LNURL:', e);
          this.$q.notify({ type: 'negative', message: 'Failed to copy LNURL.' });
        }
      },

      async writeNfcTag() {
        try {
          if (!this.nfcSupported) {
            throw {
              toString: function () {
                return 'NFC not supported on this device or browser.';
              }
            };
          }
          var ndef = new NDEFReader();
          this.nfcTagWriting = true;
          this.$q.notify({
            message: 'Tap your NFC tag to write the LNURL-withdraw link to it.'
          });
          await ndef.write({
            records: [{ recordType: 'url', data: this.lnurl, lang: 'en' }]
          });
          this.nfcTagWriting = false;
          this.$q.notify({ type: 'positive', message: 'NFC tag written successfully.' });
        } catch (error) {
          this.nfcTagWriting = false;
          this.$q.notify({
            type: 'negative',
            message: error ? error.toString() : 'An unexpected error has occurred.'
          });
        }
      },

      copyCardImage: function (purpose) {
        var canvas = this.$refs.brandedCanvas || this.$refs.qrCanvas;
        var dataUrl = this.getCardDataUrl();
        if (!canvas || !dataUrl) {
          this.$q.notify({ type: 'negative', message: 'Gift card image is not ready.' });
          return;
        }

        var isPrint = purpose === 'print';
        this.imageDialogTitle = isPrint ? 'Print Gift Card' : 'Save Gift Card';
        this.imageDialogInstructions = isPrint
          ? 'Right-click the image, open it in a new tab, then press Ctrl+P or Command+P.'
          : 'Right-click the image and select “Save image as…” to save it.';
        this.imageDialogUrl = dataUrl;
        this.imageDialogShow = true;

        var self = this;
        canvas.toBlob(function (blob) {
          if (!blob || !navigator.clipboard || !navigator.clipboard.write || typeof ClipboardItem === 'undefined') {
            return;
          }
          navigator.clipboard.write([
            new ClipboardItem({'image/png': blob})
          ]).then(function () {
            self.imageDialogInstructions = isPrint
              ? 'Image copied. Paste it into a document or image editor, then print it. You can also right-click the preview and open it in a new tab.'
              : 'Image copied. Paste it into an image editor to save it, or right-click the preview and select “Save image as…”.';
            self.$q.notify({ type: 'positive', message: 'Gift card image copied to clipboard.' });
          }).catch(function () {});
        }, 'image/png');
      },

      printCard: function () {
        this.copyCardImage('print');
      },

      downloadCard: function () {
        this.copyCardImage('save');
      },

      getCardDataUrl: function () {
        // Use the branded canvas if available, otherwise the QR canvas
        var canvas = this.$refs.brandedCanvas || this.$refs.qrCanvas;
        if (!canvas) return null;
        try {
          return canvas.toDataURL('image/png');
        } catch (e) {
          return null;
        }
      },

      renderQR: function () {
        if (!this.giftCard || this.giftCard.status !== 'active' || !this.lnurl) return;
        var self = this;

        if (this.giftCard.hasDesign) {
          this.renderBrandedCard();
        } else {
          this.$nextTick(function () {
            var canvas = self.$refs.qrCanvas;
            if (canvas && window.QRCode) {
              window.QRCode.toCanvas(canvas, self.lnurl, { width: 250 }, function () {});
            }
          });
        }
      },

      renderBrandedCard: function () {
        var self = this;
        this.$nextTick(function () {
          var canvas = self.$refs.brandedCanvas;
          if (!canvas) return;

          var templateName = self.giftCard.templateName || '';
          var templateAssetId = self.giftCard.templateAssetId || '';

          // Determine canvas dimensions from template
          var tw = 425, th = 650;
          if (templateName === 'portrait') { tw = 425; th = 650; }
          else if (templateName === 'landscape') { tw = 1050; th = 600; }
          else {
            var sample = SAMPLE_TEMPLATES.find(function (s) { return s.value === templateName; });
            if (sample) { tw = sample.w; th = sample.h; }
          }

          // Determine image URL. Custom templates are stored as data: URLs
          // on the card itself (the iframe CSP only allows ext-assets and
          // data: images, and there is no backend asset endpoint).
          var imgUrl = null;
          var isCustom = templateName === 'custom' && templateAssetId;
          if (isCustom) {
            imgUrl = templateAssetId;
          } else if (templateName && templateName !== 'portrait' && templateName !== 'landscape') {
            imgUrl = IMG_BASE + '/template_' + templateName + '.png';
          }

          var ctx = canvas.getContext('2d');

          if (imgUrl) {
            // Load template image, then draw QR + text overlay
            var img = new Image();
            img.onload = function () {
              // Custom template dimensions are taken from the image itself
              if (isCustom) {
                tw = img.naturalWidth;
                th = img.naturalHeight;
              }
              canvas.width = tw;
              canvas.height = th;
              ctx.drawImage(img, 0, 0, tw, th);
              self._drawQrAndTextOnCard(ctx, tw, th);
            };
            img.onerror = function () {
              // Fallback: draw a plain background
              canvas.width = tw;
              canvas.height = th;
              ctx.fillStyle = '#ebedf5';
              ctx.fillRect(0, 0, tw, th);
              self._drawQrAndTextOnCard(ctx, tw, th);
            };
            img.src = imgUrl;
          } else {
            canvas.width = tw;
            canvas.height = th;
            // Portrait/landscape: plain background
            ctx.fillStyle = '#ebedf5';
            ctx.fillRect(0, 0, tw, th);
            self._drawQrAndTextOnCard(ctx, tw, th);
          }
        });
      },

      _drawQrAndTextOnCard: function (ctx, tw, th) {
        var self = this;

        // Read QR/text placement from the card's stored config (if available),
        // falling back to defaults that match the card designer defaults.
        var qr = (self.giftCard.qrConfig && typeof self.giftCard.qrConfig === 'object') ? self.giftCard.qrConfig : {};
        var tc = (self.giftCard.textConfig && typeof self.giftCard.textConfig === 'object') ? self.giftCard.textConfig : {};

        var qrSize = Math.round(qr.qr_size || 200);
        var qrX = Math.round((qr.qr_x_frac != null ? qr.qr_x_frac : 0.1) * tw);
        var qrY = Math.round((qr.qr_y_frac != null ? qr.qr_y_frac : 0.7) * th);

        var fontSize = tc.font_size || 24;
        var fontFamily = tc.font_family || 'DejaVuSans';
        var fontColor = tc.font_color || '#000000';
        var textAlign = tc.text_align || 'left';
        var textX = Math.round((tc.text_x_frac != null ? tc.text_x_frac : 0.1) * tw);
        var textY = Math.round((tc.text_y_frac != null ? tc.text_y_frac : 0.1) * th);
        var showAmount = tc.show_amount !== false;
        var showRecipient = tc.show_recipient !== false;
        var showMessage = tc.show_message !== false;

        // Map font family names to CSS font-family strings
        var cssFontFamily = fontFamily;
        if (fontFamily === 'DejaVuSans') cssFontFamily = 'sans-serif';
        else if (fontFamily === 'DejaVuSerif') cssFontFamily = 'serif';
        else if (fontFamily === 'DejaVuSansMono') cssFontFamily = 'monospace';

        var qrCanvas = document.createElement('canvas');
        if (window.QRCode) {
          window.QRCode.toCanvas(qrCanvas, self.lnurl, { width: qrSize }, function () {
            ctx.drawImage(qrCanvas, qrX, qrY, qrSize, qrSize);

            // Draw text overlay
            ctx.font = fontSize + 'px ' + cssFontFamily;
            ctx.fillStyle = fontColor;
            ctx.textAlign = textAlign;
            var lineHeight = fontSize * 1.3;
            var y = textY + fontSize;

            if (showAmount) {
              ctx.fillText((self.giftCard.amount || 0) + ' sats', textX, y);
              y += lineHeight;
            }
            if (showRecipient && self.giftCard.recipientName) {
              ctx.fillText('For: ' + self.giftCard.recipientName, textX, y);
              y += lineHeight;
            }
            if (self.giftCard.senderName) {
              ctx.fillText('From: ' + self.giftCard.senderName, textX, y);
              y += lineHeight;
            }
            if (showMessage && self.giftCard.message) {
              ctx.fillText(self.giftCard.message, textX, y);
            }

            // LNURL string at bottom
            y = th - 20;
            ctx.font = '11px sans-serif';
            ctx.fillStyle = '#666';
            ctx.textAlign = 'center';
            var url = self.lnurlString || '';
            if (url.length > 55) url = url.substring(0, 52) + '...';
            ctx.fillText(url, tw / 2, y);
          });
        }
      },

      async loadGiftCard() {
        this.loading = true;
        try {
          // Connect to the bridge first (required for getRouteParams)
          await window.LNbitsBridge.connect();

          // Get raw token from route params (provided by the bridge)
          // The redeem page runs inside an iframe, so window.location.pathname
          // is the frame URL, not the redeem URL. Route params are the
          // reliable way to get the {token} from the URL path.
          var routeParams = await window.LNbitsBridge.getRouteParams();
          var rawToken = routeParams.token || '';

          // Fallback: try extracting from window.location.pathname
          // (works when the page is accessed directly, not in an iframe)
          if (!rawToken) {
            var pathParts = window.location.pathname.split('/');
            for (var i = 0; i < pathParts.length; i++) {
              if (pathParts[i] === 'redeem' && i + 1 < pathParts.length) {
                rawToken = pathParts[i + 1];
                break;
              }
            }
          }

          if (!rawToken || rawToken === 'redeem') {
            this.giftCard = null;
            return;
          }

          // Compute SHA-256 hash in the browser
          this.tokenHash = await this.computeSHA256(rawToken);

          // Load public card data via bridge
          var card = await apiCall('GET', '/cards/public/' + this.tokenHash, null);

          if (card && !card.error && !card.detail) {
            this.giftCard = card;
          } else {
            this.giftCard = null;
          }
        } catch (error) {
          console.error('Failed to load gift card:', error);
          this.giftCard = null;
        } finally {
          this.loading = false;
        }
      },

      async computeSHA256(message) {
        var msgBuffer = new TextEncoder().encode(message);
        var hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
        var hashArray = Array.from(new Uint8Array(hashBuffer));
        var hashHex = hashArray.map(function (b) {
          return b.toString(16).padStart(2, '0');
        }).join('');
        return hashHex;
      },

      formatDate: function (dateString) {
        if (!dateString) return '';
        // expiresAt / expiredAt may be a unix timestamp string
        var date;
        if (/^\d+$/.test(dateString)) {
          date = new Date(parseInt(dateString, 10) * 1000);
        } else {
          date = new Date(dateString);
        }
        return date.toLocaleDateString();
      }
    }
  });

  app.use(Quasar);
  app.mount('#q-app');
})();
