#!/usr/bin/env python3
"""Fix the online NSIS template: !insertmacro typo + description block placement."""
p = '/home/z/my-project/scripts/installer-online-template.nsi'
s = open(p).read()

# fix 1: typo !insert -> !insertmacro
assert '!insert MUI_LANGUAGE' in s
s = s.replace('!insert MUI_LANGUAGE', '!insertmacro MUI_LANGUAGE', 1)

# fix 2: the MUI_DESCRIPTION block references ${SEC_CORE}/${SEC_MEDIA}, which
# are only defined by the Section statements -> it must come AFTER all sections
desc_block = '''; ---- component descriptions (shown on the components page) ----
LangString DESC_CORE ${LANG_ENGLISH} "Velox itself: the download manager app, browser extension and native messaging host. Required."
LangString DESC_MEDIA ${LANG_ENGLISH} "YouTube Video Downloader support: downloads yt-dlp and LGPL FFmpeg (~120 MB) from their official sources during installation and installs them beside Velox. Needed for downloading and merging YouTube video/audio; leave unchecked to skip the download."

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_CORE} $(DESC_CORE)
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_MEDIA} $(DESC_MEDIA)
!insertmacro MUI_FUNCTION_DESCRIPTION_END
'''
assert desc_block in s, 'description block not found'
s = s.replace(desc_block, '', 1)
anchor = 'Section "un.Uninstall"'
assert anchor in s
s = s.replace(anchor, desc_block + '\n' + anchor, 1)
open(p, 'w').write(s)
print('fixed: !insertmacro typo + description block after sections')
