# DoOnce — Chrome’da görünür hale getirme

Native host (`doonce setup`) **sadece arka plandaki motoru** bağlar. Arayüz **Chrome extension** ile gelir; ikisi ayrı adımlardır.

## 1. Extension’ı yükle

1. Chrome’da aç: `chrome://extensions`
2. Sağ üstte **Geliştirici modu** açık olsun
3. **Paketlenmemiş öğe yükle**
4. Şu klasörü seç (içindeki `dist`, üst klasör değil):

   `/Users/selcukyildirim/projects/DoOnce/extension/dist`

5. Listede **DoOnce** satırı görünmeli (mavi yuvarlak ikon)

## 2. Araç çubuğuna sabitle

Extension yüklü olsa bile simge görünmeyebilir:

1. Chrome adres çubuğunun sağındaki **yapboz parçası** (Extensions)
2. **DoOnce** yanındaki **iğne** simgesine tıkla
3. Araç çubuğunda mavi **DoOnce** ikonu belirir

## 3. Paneli aç

- Araç çubuğundaki **DoOnce** ikonuna tıkla → sağda **yan panel** açılır
- “DoOnce 0.1.0 · …/DoOnce” yazıyorsa host bağlantısı tamam

## Sık hatalar

| Belirti | Çözüm |
|--------|--------|
| Extensions listesinde DoOnce yok | `extension/dist` seçildiğinden emin ol; `npm run build` çalıştır |
| DoOnce var ama simge yok | Yapboz menüsünden **Pin** |
| Panel boş | Extension’da **Yenile**; `extension` içinde tekrar `npm run build` |
| “Not connected” | `./scripts/install-host.sh` sonra extension **Yenile** |
| Host hatası | `chrome://extensions` → DoOnce → **Hata** / Service worker “Inspect” |

## Extension kimliği

Paketlenmemiş yüklemede manifest `key` sayesinde id sabittir:

`mmninaegachgmioebddmlccpjdkcdefb`

Bu id, `doonce setup` manifest’indeki `allowed_origins` ile eşleşmelidir.
