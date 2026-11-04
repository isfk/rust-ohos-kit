// The list, as plain rows rather than a List: it lives inside the page's Scroll,
// and a nested scrollable eats the gesture that would reach the panels below it
// (measured — a List at 220vp filled the whole viewport).
//
// No state of its own: the page owns the todos and this renders them. Toggles and
// deletes go out as events, the page decides what they mean. The count moved up
// to the panel header when the list moved under its own input — the page already
// owns the array, so it counts it there.

import { Todo } from '../../core/RustCore';

@ComponentV2
export struct TodoList {
  @Param todos: Todo[] = [];
  @Event onToggle: (id: number) => void = (id: number): void => {};
  @Event onRemove: (id: number) => void = (id: number): void => {};

  build() {
    Column() {
      if (this.todos.length === 0) {
        Text('Nothing yet')
          .fontSize(13)
          .fontColor($r('app.color.text_muted'))
          .padding({ top: 4, bottom: 4 })
      }
      ForEach(this.todos, (t: Todo, i: number) => {
        Column() {
          // One row height for every row, so the checkbox column, the titles
          // and the delete action all sit on the same lines.
          Row({ space: 12 }) {
            Toggle({ type: ToggleType.Checkbox, isOn: t.done })
              .width(24)
              .height(24)
              .selectedColor($r('app.color.accent'))
              .onChange(() => {
                this.onToggle(t.id);
              })
            Text(t.title)
              .fontSize(15)
              .fontColor(t.done ? $r('app.color.text_muted') : $r('app.color.text_primary'))
              .layoutWeight(1)
              .maxLines(2)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
              .decoration({
                type: t.done ? TextDecorationType.LineThrough : TextDecorationType.None,
                color: $r('app.color.text_muted'),
              })
            // Quiet on purpose: a red button on every row shouts twelve times.
            Text('Delete')
              .fontSize(13)
              .fontColor($r('app.color.text_muted'))
              .padding({ left: 8, right: 2, top: 10, bottom: 10 })
              .onClick(() => {
                this.onRemove(t.id);
              })
          }
          .width('100%')
          // A wrapped title needs a second line, so this is a floor and not a
          // fixed height: the row grows for two lines of text and no further.
          .constraintSize({ minHeight: 48 })
          .padding({ top: 6, bottom: 6 })

          // Inset to the title column and never under the last row: a line that
          // starts at the card edge reads as a table, and one that dangles
          // below the final row is a rule with nothing to separate.
          if (i < this.todos.length - 1) {
            Divider()
              .strokeWidth(1)
              .color($r('app.color.hairline'))
              .margin({ left: 36 })
          }
        }
        .width('100%')
      }, (t: Todo) => t.id.toString())
    }
    .width('100%')
    .alignItems(HorizontalAlign.Start)
  }
}
