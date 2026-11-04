// A photo, end to end: the platform takes it and decodes it, the core does the
// maths, the UI draws the result.
//
// The split is the point. HarmonyOS's decoder reads every format the camera
// writes (HEIF included) and can scale — that's not the core's business. The
// core gets plain RGBA and answers questions about light: how bright, where,
// and out of which colours.
//
// The result is drawn on its own dark bed, the way a lightbox or an instrument
// display is: the numbers are light-on-dark in both themes, so a cell's alpha
// can mean brightness and nothing else. That is also why the colours below are
// literals instead of theme tokens — this block must not follow the page theme.

import { cameraPicker, camera } from '@kit.CameraKit';
import { image } from '@kit.ImageKit';
import { fileIo as fs } from '@kit.CoreFileKit';
import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { AnalyzeResult, ColorShare, FileInfo, rustcore, ImageAnalysis } from '../../core/RustCore';

/** Photos are decoded to this on the wide side: 12 MP would only cost time. */
const ANALYSIS_WIDTH = 320;

/** The analysis bed: deliberately theme-independent (see the file header). */
const BED_BG = '#0B0D14';
const BED_LABEL = '#8A93A5';
const BED_META = '#6C7586';
const BED_TEXT = '#C9D1DE';
const BED_BAR = '#38FFFFFF';
const BED_TALLEST = '#00ADD8';
const BED_RULE = '#24FFFFFF';

/** 8 cells of 14 plus 7 gaps of 1: the light map's own width. */
const GRID_SIDE = 119;
/** 8 bars of 16 plus 7 gaps of 4: the levels' own width. */
const BAR_SIDE = 156;

@ComponentV2
export struct PhotoCard {
  @Local private file: FileInfo | undefined = undefined;
  @Local private analysis: ImageAnalysis | undefined = undefined;
  /** Empty until something happened: the help line below says the rest. */
  @Local private status: string = '';
  @Local private busy: boolean = false;

  private async fromGallery(): Promise<void> {
    try {
      const picker = new photoAccessHelper.PhotoViewPicker();
      const chosen = await picker.select({
        MIMEType: photoAccessHelper.PhotoViewMIMETypes.IMAGE_TYPE,
        maxSelectNumber: 1,
      });
      if (chosen.photoUris.length === 0) {
        return;
      }
      await this.analyze(chosen.photoUris[0]);
    } catch (err) {
      this.status = `${(err as BusinessError).message}`;
    }
  }

  /** The system's camera UI: it takes the picture, we get a URI back. */
  private async fromCamera(): Promise<void> {
    if (this.busy) {
      return;
    }
    this.busy = true;
    try {
      const res = await cameraPicker.pick(
        getContext(this), [cameraPicker.PickerMediaType.PHOTO],
        { cameraPosition: camera.CameraPosition.CAMERA_POSITION_BACK });
      if (res.resultCode !== 0 || res.resultUri.length === 0) {
        this.status = res.resultCode === 0 ? 'no photo taken' : `camera picker: ${res.resultCode}`;
        return;
      }
      await this.analyze(res.resultUri);
    } catch (err) {
      this.status = `${(err as BusinessError).message}`;
    } finally {
      this.busy = false;
    }
  }

  /** One photo: what the file is (the core reads it), then what it looks like. */
  private async analyze(uri: string): Promise<void> {
    this.status = 'reading…';
    const parts = uri.split('/');
    const name = parts.length > 0 ? parts[parts.length - 1] : 'photo';
    const file = fs.openSync(uri, fs.OpenMode.READ_ONLY);
    try {
      const inspected = rustcore.inspect(file.fd, name);
      this.file = inspected.ok ? inspected.file : undefined;

      // The platform decodes and scales; the core never sees a JPEG.
      const source = image.createImageSource(file.fd);
      const info = await source.getImageInfo();
      const scale = Math.min(1, ANALYSIS_WIDTH / Math.max(info.size.width, info.size.height));
      const width = Math.max(1, Math.round(info.size.width * scale));
      const height = Math.max(1, Math.round(info.size.height * scale));
      const pixels = await source.createPixelMap({
        desiredSize: { width: width, height: height },
        desiredPixelFormat: image.PixelMapFormat.RGBA_8888,
      });
      const buffer = new ArrayBuffer(width * height * 4);
      await pixels.readPixelsToBuffer(buffer);
      pixels.release();
      source.release();

      const result: AnalyzeResult = rustcore.analyze(buffer, width, height);
      if (!result.ok || result.image === undefined) {
        this.analysis = undefined;
        this.status = result.error ?? 'the core could not read the pixels';
        return;
      }
      this.analysis = result.image;
      const a = result.image;
      const kb = (buffer.byteLength / 1024).toFixed(0);
      const ms = (a.micros / 1000).toFixed(1);
      this.status = `${a.width}×${a.height} · ${kb} KB RGBA · ${a.meanLuma}% bright · ${ms} ms in Rust`;
    } catch (err) {
      this.analysis = undefined;
      this.status = `decode: ${(err as BusinessError).message}`;
    } finally {
      fs.closeSync(file);   // the descriptor was only borrowed
    }
  }

  private fileLine(): string {
    const f = this.file;
    if (f === undefined) {
      return '';
    }
    const mb = (f.bytes / (1024 * 1024)).toFixed(1);
    return `${f.kind} · ${mb} MB · ${f.sha256.substring(0, 12)}…`;
  }

  /** Which luma bucket holds the most pixels: that bar carries the accent. */
  private tallest(a: ImageAnalysis): number {
    let at = 0;
    for (let i = 0; i < a.histo.length; i++) {
      if (a.histo[i] > a.histo[at]) {
        at = i;
      }
    }
    return at;
  }

  /**
   * 8×8 squares: where the light falls, not one average for the whole frame.
   * The cell is white at the brightness the core measured, so a bright corner
   * reads bright — the panel is the dark bed it sits on.
   */
  @Builder
  lightGrid(a: ImageAnalysis) {
    Column({ space: 1 }) {
      ForEach([0, 1, 2, 3, 4, 5, 6, 7], (row: number) => {
        Row({ space: 1 }) {
          ForEach([0, 1, 2, 3, 4, 5, 6, 7], (col: number) => {
            Row()
              .width(14)
              .height(14)
              .borderRadius(3)
              .backgroundColor(`rgba(255, 255, 255, ${(a.grid[row * 8 + col] / 100 * 0.92).toFixed(2)})`)
          }, (col: number) => `c${row}-${col}`)
        }
      }, (row: number) => `r${row}`)
    }
    .width(GRID_SIDE)
  }

  /** The same numbers as bars: darkest on the left, brightest on the right. */
  @Builder
  histogram(a: ImageAnalysis) {
    Column({ space: 6 }) {
      Row({ space: 4 }) {
        ForEach(a.histo, (pct: number, i: number) => {
          Column() {
            Blank()
            Row()
              .width(16)
              .height(Math.max(2, pct) * 0.56)   // 100% → 56pt tall
              .borderRadius(2)
              .backgroundColor(this.tallest(a) === i ? BED_TALLEST : BED_BAR)
          }
          .height(56)
          .justifyContent(FlexAlign.End)
        }, (pct: number, i: number) => `b${i}`)
      }
      .height(56)

      Row()
        .width(BAR_SIDE)
        .height(1)
        .backgroundColor(BED_RULE)

      Row() {
        Text('dark')
          .fontSize(11)
          .fontColor(BED_LABEL)
        Blank()
        Text('bright')
          .fontSize(11)
          .fontColor(BED_LABEL)
      }
      .width(BAR_SIDE)
    }
    .alignItems(HorizontalAlign.Start)
  }

  /** The picture's own three colours: the only saturated thing on the page. */
  @Builder
  palette(a: ImageAnalysis) {
    Row({ space: 12 }) {
      ForEach(a.colors, (c: ColorShare) => {
        Row({ space: 6 }) {
          Row()
            .width(14)
            .height(14)
            .borderRadius(7)
            .backgroundColor(c.hex)
            .border({ width: 1, color: BED_RULE })
          Text(`${Math.round(c.share * 100)}%`)
            .fontSize(12)
            .fontColor(BED_TEXT)
            .fontFeature('"tnum" 1')
        }
        .alignItems(VerticalAlign.Center)
      }, (c: ColorShare) => c.hex)
    }
  }

  @Builder
  analysisBed(a: ImageAnalysis) {
    Column({ space: 12 }) {
      Row({ space: 16 }) {
        Column({ space: 6 }) {
          Text('Light')
            .fontSize(11)
            .fontColor(BED_LABEL)
          this.lightGrid(a)
        }
        .alignItems(HorizontalAlign.Start)

        Column({ space: 12 }) {
          Column({ space: 6 }) {
            Text('Levels')
              .fontSize(11)
              .fontColor(BED_LABEL)
            this.histogram(a)
          }
          .alignItems(HorizontalAlign.Start)

          Column({ space: 6 }) {
            Text('Palette')
              .fontSize(11)
              .fontColor(BED_LABEL)
            this.palette(a)
          }
          .alignItems(HorizontalAlign.Start)
        }
        .alignItems(HorizontalAlign.Start)
        .layoutWeight(1)
      }
      .width('100%')
      .alignItems(VerticalAlign.Top)

      if (this.fileLine().length > 0) {
        Text(this.fileLine())
          .fontSize(11)
          .fontColor(BED_META)
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .width('100%')
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
    .padding(12)
    .backgroundColor(BED_BG)
    .borderRadius(8)
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 8 }) {
        Text('Photo')
          .fontSize(15)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.text_primary'))
        Blank()
        Button('Gallery')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 12, right: 12 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .onClick(() => {
            this.fromGallery();
          })
        Button('Take photo')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 12, right: 12 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .enabled(!this.busy)
          .onClick(() => {
            this.fromCamera();
          })
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      if (this.analysis !== undefined) {
        this.analysisBed(this.analysis)
      } else {
        Text('Choose a photo to read its light and its colours')
          .fontSize(13)
          .fontColor($r('app.color.text_secondary'))
          .width('100%')
      }

      if (this.status.length > 0) {
        Text(this.status)
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .fontFeature('"tnum" 1')
          .maxLines(2)
          .width('100%')
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
    .padding(16)
    .backgroundColor($r('app.color.surface'))
    .borderRadius(12)
  }
}
