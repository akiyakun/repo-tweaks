"""トークンの比較"""

import hmac


def same_token(a, b):
    return bool(a) and bool(b) and hmac.compare_digest(str(a), str(b))
