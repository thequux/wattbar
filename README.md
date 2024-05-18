# Wattbar

Wattbar is a minimalist battery charge monitor. It simply draws the battery
level in a narrow strip along the bottom of the screen.


## History
Wattbar is only the latest in a long line of battery monitors starting with
[YAMAGUCHI Suguru's xbattbar](https://github.com/lichtblau/xbattbar) in 1998.

The original program has been updated by a number of different authors to support
more modern hardware interfaces; the original APM was already getting replaced by
ACPI in 2005, and the kernel interfaces to access ACPI information have changed
significantly in the time since. Thus, in 2015, I (TQ Hirsch) rewrote the utility
from scratch in Go; this version was called [xbattbar3](https://github.com/thequux/xbattbar3);
to avoid kernel interface churn, I used UPowerd to access the battery charge status.
(For the curious, I consider the ACPI version linked above to be xbattbar2, thus the
next version is version 3)

In 2022, X has been starting to get a bit long in the tooth, and my laptop works
far better with Wayland than X. Thus, it became time to rewrite it again; this new
version needed a different name. "wbattbar" was the initial obvious choice, but
leaving out the "b" made it a better pun. Aside from the fact that it renders to
Wayland instead of X11, it should be a drop-in replacement for any of the previous
versions of xbattbar.

Wattbar offers a modern Wayland replacement of xbattbar of a battery status 
monitoring UPowerd representing a bar that fills one side of the screen with a 
smooth change of selected colors that can be set in a cofiguration theme file.

## Building from sources with RUST:
Clone the repository:
```
git clone https://github.com/thequux/wattbar.git && cd wattbar
```
Compile using cargo:
```
cargo install --path ./
```
The binary is installed in ```$HOME/.cargo/bin/wattbar```. Be sure is on ```$PATH```.

## Configuration:
Config file template is stored in ```$HOME/.config/wattbar/default.theme```, copy/rename 
and create as many themes as you like. Inside this file you can setup three modesets 
for the battery to use depending on its status:

```
[status]
Charge_%	Color_code_representation	Background_color_code_representation
```

Status modes are:
[charging], [discharging] and [nocharge] that activates the color of the bar when 
the battery reports any of this modes.

Colors can be set using plain RGB representation by #RRGGBB input color code mode 
or cilindrical HSL representation color code mode by hsl(Hue, Saturation_%, Lightness_%).

Wattbar also comes with gradient color transformation for both Color_code and
Background_color that enables smooth color transformation between Charge_%'s segments.

If no Background color code is set, Wattbar automatically sets 50% darker color
of the Color_code for the Background_color setting.

Examples of themes for a bluish-light

```
[charging]
0%	#0000FF
25%	#000044 #555588
50%	#000044	#EEEEFF
100%	#0000FF #FFDDFF

[discharging]
0%	#000044	#0000FF
25%	#FFFF00 #8888AA
50%	#000044
100%	#0000FF

[nocharge]
0%	#000044 #EEEEFF
```

and redish-dark theme:

```
[charging]
0%	#FF00FF #220022
25%	#FFFF00 #880088
50%	#FF0000	#880000
100%	#FF00FF #880088

[discharging]
0%	#440000	#110011
25%	#FFFF00 #880000
50%	#FF0000
100%	#FF00FF

[nocharge]
0%	#AA0000 #440000
```

## Flags:
Next combination of flags that can come in handy to personalize Wattbar:

Usage: ```wattbar [OPTIONS]```

Options:
*  ```-b, --border <BORDER>```  Which border to draw the bar on. One of left, right, top, 
  or bottom (or l,r,t, or b) [default: bottom]
*  ```-s, --size <SIZE>```      How many virtual pixels tall the bar should be [default: 3]
*  ```-r, --reverse```          Reverse the direction of the bar (i.e., right-to-left, or 
  top-to-bottom)
*  ```-t, --theme <THEME>```    The theme to use. Passing a non-existent theme will tell you 
  where wattbar looks [default: default]
*  ```-h, --help```             Print help
*  ```-V, --version```          Print version

**To test discharging color change of the selected theme (or default if no theme is chosen), you 
can use the _supersecretflag_ ```--mock-upower```.**

